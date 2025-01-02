//! Generate ready-to-flash raw images with AOSC OS for various devices
//!
//! Requirements
//! ------------
//!
//! The following dependencies are required to build and run this tool:
//!
//! ### Library Dependencies (Linked Libraries)
//!
//! - `libblkid`: for gathering information for block devices, primarily their unique identifiers.
//! - `liblzma`: for compressing the image file with LZMA2 (xz).
//! - `libzstd`: for compressing the image file with ZStandard.
//!
//! ### Runtime Dependencies (External commands)
//!
//! The following executables must be available in the system at runtime:
//!
//! - `rsync`: For copying the system distribution.
//! - `mkfs.ext4`, `mkfs.xfs`, `mkfs.btrfs`, `mkfs.vfat`: For making filesystems on partitions.
//! - `chroot`: For entering the chroot environment of the target container to perform post-installation steps.
//! - `useradd` from shadow: For adding user to the target container.
//! - `chpasswd` from shadow: For changing user passwords.
//! - `partprobe`: For updating the in-kernel partition table cache.
//!
//! ### `binfmt_misc` support and respective binary interpreters
//!
//! If you intend to build images for devices with a different architecture than your host machine, you must check if your host system supports `binfmt_misc`:
//!
//! ```shell
//! $ cat /proc/sys/fs/binfmt_misc/status
//! enabled
//! ```
//!
//! <div class="warning">
//!
//! Enabling `binfmt_misc` support is beyond the scope of this documentation.
//!
//! </div>
//!
//! With `binfmt_misc` support enabled, you will have to install `qemu-user-static` (or equivalent packages for your distribution) to allow your system to execute binary executables for the target device's architecture.
//!
//! Building
//! --------
//!
//! Simply run:
//!
//! ```shell
//! cargo build --release
//! ```
//! Usage
//! -----
//!
//! ### List Available Devices
//!
//! ```shell
//! $ ./target/release/mkrawimg list --format FORMAT
//! ```
//!
//! While `FORMAT` can be one of the following:
//!
//! - `pretty`: table format which contains basic information.
//! - `simple`: simple column-based format splitted by tab character (`'\t'`).
//!
//! ### Build images for one specific device
//!
//! <div class="warning">
//! Building images requires the root privileges.
//! </div>
//!
//! ```shell
//! # ./target/release/mkrawimg build --variants VARIANTS DEVICE
//! ```
//!
//! - `VARIANTS`: distribution variants, can be one or more of the `base`, `desktop`, `server`.
//!   If not specified, all variants will be built.
//! - `DEVICE`: A string identifying the target device, can be one of the following:
//!   - Device ID (defined in `device.toml`).
//!   - Device alias (defined in `device.toml`).
//!   - The path to the `device.toml` file.
//!
//! ### Build Images for All Devices (in the registry)
//!
//! ```shell
//! # ./target/release/mkrawimg build-all --variants VARIANTS
//! ```
//!
//! For the advanced usage, please refer to [`Cmdline`].
//!
//! Adding a new device
//! -------------------
//!
//! To add support for a new device, please refer to [`DeviceSpec`].
//!
//! Contributing
//! ------------
//!
//! ### Device addition
//!
//! While CI performs automated checks on submitted device specification files, these checks are not exhaustive. Therefore, we require you to build an image using your specification file to ensure its validity.
//!
//! License
//! -------
//!
//! This repository is licensed under the GNU GPL v3 license.
//!
// #![allow(warnings)]
// Why do you guys hate tabs?
// Look, I use tabs for indentation in my code.
// I have some sample code from the Linux kernel in my docstrings.
// Clippy warns me about the tabs, this is denial!
#![allow(clippy::tabs_in_doc_comments)]
mod bootloader;
mod cli;
/// Module handling the actual generation jobs.
#[doc(hidden)]
mod context;
/// Module handling various procedures for a specific device, and the device specification itself.
mod device;
/// Module handling the filesystems.
#[doc(hidden)]
mod filesystem;
/// Module handling the partitions.
mod partition;
/// Module handling the package installation.
#[doc(hidden)]
mod pm;
mod registry;
#[doc(hidden)]
mod tests;
#[doc(hidden)]
mod topics;
/// Module containing various utility functions.
#[doc(hidden)]
mod utils;

pub use cli::Cmdline;
pub use device::DeviceSpec;
use topics::{fetch_topics, filter_topics};

use core::time;
use std::{
	env::var,
	fs::{remove_dir, remove_dir_all},
	path::{Path, PathBuf},
	time::Instant,
};

use anyhow::bail;
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use clap::Parser;
use cli::Action;
use cli::RootFsType;
use context::{ImageContext, ImageContextQueue};
use filesystem::FilesystemType;
use log::{debug, error, info, warn};
use owo_colors::colored::*;
use registry::DeviceRegistry;
use utils::{bootstrap_distribution, check_binfmt, restore_term, return_ownership_recursive};

#[doc(hidden)]
enum BuildMode {
	BuildOne,
	BuildAll,
	None, // check
}

#[doc(hidden)]
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

	std::env::set_var("LANG", "C");
	std::env::set_var("LC_ALL", "C");

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

#[doc(hidden)]
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
			topics,
			..
		}
		| cli::Action::BuildAll {
			fstype,
			compression: compress,
			variants,
			revision,
			additional_packages,
			topics,
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
			let topics = if let Some(topics) = topics.as_ref() {
				let all_topics = fetch_topics()?;
				let filtered_topics = filter_topics(topics, all_topics)?;
				Some(filtered_topics)
			} else {
				None
			};
			let topics = topics.as_ref();
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
						topics,
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
			if cmdline.cleanup {
				info!("Cleaning up the sketch directories ...");
				let sketch_dir = cmdline.workdir.join("sketches");
				match remove_dir_all(&sketch_dir) {
					Ok(_) => (),
					Err(e) => {
						warn!("Unable to remove the directory {}: {}\nYou have to remove them manually.", &sketch_dir.display(), e);
					}
				}
			}
			if cmdline.cleanup_bootstrap {
				info!("Cleaning up the bootstrapped system distributions ...");
				let bootstrap_dir = cmdline.workdir.join("bootstrap");
				match remove_dir_all(&bootstrap_dir) {
					Ok(_) => (),
					Err(e) => {
						warn!("Unable to remove the directory {}: {}\nYou have to remove them manually.", &bootstrap_dir.display(), e);
					}
				}
			}
			if cmdline.cleanup && cmdline.cleanup_bootstrap {
				info!("Removing the working directory ...");
				match remove_dir(&cmdline.workdir) {
					Ok(_) => (),
					Err(e) => {
						warn!("Unable to remove the working directory {}: {}\nYou have to remove them manually.", &cmdline.workdir.display(), e);
					}
				}
			}
			let uid = var("SUDO_UID").ok();
			let gid = var("SUDO_GID").ok();
			if uid.is_some() || gid.is_some() {
				let uid = uid
					.map(|x| {
						x.parse::<u32>().with_context(|| {
							format!("Failed to parse $SUDO_UID '{}' into integer",
						&x)
						})
					})
					.transpose()?;
				let gid = gid
					.map(|x| {
						x.parse::<u32>().with_context(|| {
							format!("Failed to parse $SUDO_GID '{}' into integer",
						&x)
						})
					})
					.transpose()?;
				info!("This tool is running with sudo, fixing ownership of the output directory ...");
				return_ownership_recursive(&cmdline.outdir, uid, gid)?;
			}
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
