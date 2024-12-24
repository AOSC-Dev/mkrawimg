// #![allow(warnings)]
// Why do you guys hate tabs?
// Look, I use tabs for indentation in my code.
// I have some sample code from the Linux kernel in my docstrings.
// Clippy warns me about the tabs, this is denial!
#![allow(clippy::tabs_in_doc_comments)]
mod bootloader;
mod cli;
mod context;
mod device;
mod filesystem;
mod partition;
mod pm;
mod registry;
mod tests;
mod utils;

use core::time;
use std::{
	path::{Path, PathBuf},
	time::Instant,
};

use anyhow::bail;
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use clap::Parser;
use cli::Action;
use cli::{Cmdline, RootFsType};
use context::{ImageContext, ImageContextQueue};
use filesystem::FilesystemType;
use log::{debug, error, info, warn};
use owo_colors::colored::*;
use registry::DeviceRegistry;
use utils::{bootstrap_distribution, check_binfmt, restore_term};

enum BuildMode {
	BuildOne,
	BuildAll,
	None, // check
}

const DISTRO_REGISTRY_DIR: &str = match option_env!("DISTRO_REGISTRY_DIR") {
	Some(x) => x,
	_ => "/usr/share/aosc-mkrawimg/devices",
};

fn main() -> Result<()> {
	ctrlc::set_handler(move || {
		restore_term();
		eprintln!("\nReceived Ctrl-C, exiting.");

		std::process::exit(1);
	})
	.context("Can not register Ctrl-C (SIGTERM) handler.")?;

	// Parse the command line
	let cmdline = Cmdline::try_parse()?;
	match &cmdline.action {
		Action::Build { .. } | Action::BuildAll { .. } => {
			if unsafe { utils::geteuid() } != 0 {
				bail!("Please run me as root!");
			}
		}
		_ => (),
	}
	let mut logger = colog::basic_builder();
	if cmdline.debug {
		logger.filter(None, log::LevelFilter::Debug);
	} else {
		logger.filter(None, log::LevelFilter::Info);
	}
	logger.init();
	if cmdline.debug {
		debug!("Debug output enabled.");
	}
	if let Err(e) = try_main(cmdline) {
		// Recover the terminal
		restore_term();
		// Use logger to pretty-print errors
		let mut str_buf = String::new();
		error!("Error encountered!\n{}", e);
		let mut ident = 0;
		e.chain().skip(1).for_each(|cause| {
			let ident_str = "\t".repeat(ident);
			ident += 1;
			str_buf += &format!("{0}- Caused by:\n{0}  {1}", ident_str, cause);
		});
		if !str_buf.is_empty() {
			error!("{}", str_buf);
		}
		error!("Exiting now.");
		std::process::exit(1);
	}
	Ok(())
}

fn try_main(cmdline: Cmdline) -> Result<()> {
	// Say hi
	info!("Welcome to mkrawimg!");
	// Operation mode: build, buildall, test.
	let action = cmdline.action;
	let mut buildmode = BuildMode::None;
	// let mut devices = Vec::new();
	let registry_dir = if let Some(path) = cmdline.registry {
		path
	} else if PathBuf::from("./devices").exists() {
		PathBuf::from("./devices")
	} else {
		PathBuf::from(DISTRO_REGISTRY_DIR)
	};

	let registry_dir = if !registry_dir.exists() {
		Err(anyhow!(
			"Specified registry '{}' does not exist.",
			registry_dir.to_string_lossy()
		))
	} else if !registry_dir.is_dir() {
		Err(anyhow!(
			"Specified registry '{}' is not a directory.",
			registry_dir.to_string_lossy()
		))
	} else {
		registry_dir.canonicalize().context(format!(
			"Registry path '{}' can not be canonicalized",
			registry_dir.to_string_lossy()
		))
	};
	let registry_dir = if let Ok(x) = registry_dir {
		x
	} else {
		return Err(anyhow!(
			"Cannot assemble registry: {}",
			registry_dir.unwrap_err().bright_red()
		));
	};
	let device_str = match &action {
		cli::Action::Build { ref device, .. } => {
			buildmode = BuildMode::BuildOne;
			Some(device.to_owned())
		}
		cli::Action::BuildAll { .. } => {
			warn!("Attempting to build images for all devices. Make sure this is what you want to do.");
			buildmode = BuildMode::BuildAll;
			None
		}
		cli::Action::Check { device } => device.as_ref().map(|d| d.to_owned()),
		cli::Action::List { .. } => None,
	};
	let registry = if let Some(device_str) = &device_str {
		let try_path = Path::new(&device_str);
		if try_path.exists() {
			DeviceRegistry::from(try_path)?
		} else if registry_dir.join(try_path).exists() {
			info!("Relative path detected, assuming it's within the registry directory.");
			DeviceRegistry::from(registry_dir.join(try_path))?
		} else {
			info!("Device ID or alias '{}' provided. Assembling the full registry ...", &device_str);
			DeviceRegistry::scan(registry_dir)?
		}
	} else {
		DeviceRegistry::scan(registry_dir)?
	};
	match action {
		cli::Action::Build {
			fstype,
			compression: compress,
			variants,
			revision,
			additional_packages,
			..
		}
		| cli::Action::BuildAll {
			fstype,
			compression: compress,
			variants,
			revision,
			additional_packages,
		} => {
			let fstype = match fstype {
				Some(RootFsType::Ext4) => Some(FilesystemType::Ext4),
				Some(RootFsType::Btrfs) => Some(FilesystemType::Btrfs),
				Some(RootFsType::Xfs) => Some(FilesystemType::Xfs),
				_ => None,
			};
			let date = Utc::now();
			let date_str = date.format("%Y%m%d");
			let devices = match buildmode {
				BuildMode::BuildAll => registry.get_all()?,
				BuildMode::BuildOne => {
					let v = vec![registry.get(device_str.as_ref().unwrap())?];
					// Since we need to try to get a device with that name first.
					info!(
						"Going to build images for device '{}'.",
						&device_str.unwrap()
					);
					v
				}
				BuildMode::None => {
					panic!("Should not go here");
				}
			};
			// Prepare to build
			info!("Preparing build ...");
			std::fs::create_dir_all(&cmdline.workdir)?;
			std::fs::create_dir_all(&cmdline.outdir)?;
			// build image contexts
			let mut queue = ImageContextQueue::new();
			let variants = variants.as_slice();
			let user = &cmdline.user;
			let password = &cmdline.password;
			for device in devices.as_slice() {
				check_binfmt(&device.arch)?;
				for variant in variants {
					let variant_str = variant.to_string().to_lowercase();
					// aosc-os_desktop_rawimg_raspberrypi_rpi-5b_20241108{.1}.img.xz
					let base_dist = Path::new(&cmdline.workdir).join(format!(
						"bootstrap/{}-{}",
						&variant_str,
						&device.arch.to_string().to_lowercase()
					));
					let filename = format!(
						"aosc-os_{0}_rawimg_{1}_{2}_{3}{4}_{5}.img{6}",
						&variant.to_string().to_lowercase(),
						&device.vendor.clone(),
						&device.id.clone(),
						&date_str,
						match revision {
							Some(x) => {
								format!(".{}", x)
							}
							_ => "".to_string(),
						},
						&device.arch.to_string().to_ascii_lowercase(),
						compress.get_extension()
					);
					queue.push(ImageContext {
						device,
						variant,
						workdir: &cmdline.workdir,
						outdir: &cmdline.outdir,
						user,
						password,
						filename,
						override_rootfs_fstype: &fstype,
						additional_packages: &additional_packages,
						compress: &compress,
						base_dist,
					});
				}
			}
			info!(
				"Job queue contains {} images for {} devices.",
				queue.len().bright_cyan(),
				devices.len().bright_cyan()
			);
			info!("Bootstrapping releases...");
			for variant in variants {
				let variant_str = variant.to_string().to_lowercase();
				for device in devices.as_slice() {
					let arch = device.arch;
					let bootstrap_path =
						Path::new(&cmdline.workdir).join(format!(
							"bootstrap/{}-{}",
							&variant_str,
							arch.to_string().to_lowercase()
						));
					if !bootstrap_path.is_dir()
						|| !(bootstrap_path.join("etc/os-release")).exists()
					{
						bootstrap_distribution(
							variant,
							bootstrap_path,
							arch,
							&cmdline.mirror,
						)?;
					}
				}
			}
			let mut count: usize = 0;
			let len = queue.len();
			info!("Begin to generate images ...");
			std::thread::sleep(time::Duration::from_secs(2));
			info!("Executing the queue ...");
			let start = Instant::now();
			for j in queue {
				info!("{} images pending.", len - count);
				count += 1;
				j.execute(count, len)?;
			}
			let duration = start.elapsed();
			info!(
				"Done! {} image(s) in {:.03} seconds.",
				len,
				duration.as_secs_f32()
			);
			info!("Output directory: {}", &cmdline.outdir.display());
			info!("Program finished successfully. Exiting.");
		}
		cli::Action::Check { .. } => {
			info!("Checking validity of the registry ...");
			registry.check_validity()?;
			return Ok(());
		}
		cli::Action::List { format } => {
			registry.list_devices(format)?;
			return Ok(());
		}
	};
	Ok(())
}
