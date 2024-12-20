//! `mod bootloader`
//! This module implements simple logic to apply bootloader images to the target raw media image.
use std::{
	fs::File,
	io::{copy, BufReader, Seek},
	path::{Path, PathBuf},
};

use anyhow::Result;
use log::debug;
use serde::Deserialize;

use crate::{context::ImageContext, utils::run_script_with_chroot};

/// The [`BootloaderSpec`] specifies how to apply a bootloader image (file) to the target image.
///
/// You can write a file (inside the filesystem) to a specific partition, or to a specific location,
/// or use a script to finish this step.
///
/// In `device.toml`, this is an optional list. The list will be executed sequencially.
///
/// Example
/// -------
///
/// ```toml
/// [[bootloader]]
/// type = script
/// # the script name, must be within the same directory as device.toml
/// name = apply-bootloader.sh
///
/// [[bootloader]]
/// type = flash_partition
/// # The path must be a valid file inside the target root filesystem
/// # symbolic links are allowed
/// path = "/usr/lib/u-boot/rk64/rk3588-orange-pi-5-max-idbloader.img"
/// partition = 1
///
/// [[bootloader]]
/// type = flash_partition
/// # The path must be a valid file inside the target root filesystem
/// # symbolic links are allowed
/// path = "/usr/lib/u-boot/rk64/rk3588-orange-pi-5-max.itb"
/// partition = 2
///
/// [[bootloader]]
/// type = flash_offset
/// path = "/path/to/bootlodaer/image"
/// offset = 0x400
/// ```
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum BootloaderSpec {
	/// Run the script within the same directory as `device.toml`
	Script { name: String },
	/// Flash a file (inside the target image) to a specific partition
	FlashPartition { path: PathBuf, partition: u64 },
	/// Flash a file (inside the target image) to a specfifc location at the image
	FlashOffset { path: PathBuf, offset: u64 },
}

impl BootloaderSpec {
	fn run_script<P, Q>(container: P, script: Q) -> Result<()>
	where
		P: AsRef<Path>,
		Q: AsRef<Path>,
	{
		let container = container.as_ref();
		let script = script.as_ref();
		debug!("Copying the bootloader script ...");
		let filename = script.file_name().unwrap();
		let dst = container.join("tmp").join(filename);
		std::fs::copy(script, dst)?;
		run_script_with_chroot(container, &Path::new("/tmp").join(filename), None)
	}

	fn apply_offset<P, Q, R>(img: P, offset: u64, container: Q, loopdev: R) -> Result<()>
	where
		P: AsRef<Path>,
		Q: AsRef<Path>,
		R: AsRef<Path>,
	{
		let img = img.as_ref();
		let container = container.as_ref();
		let loopdev = loopdev.as_ref();
		// Users want to specify absolute paths. However join()ing with an absolute path replaces the whole path.
		let img_canon = container.join(img.to_string_lossy().trim_start_matches('/'));
		let img_fd = File::options().read(true).create(false).open(&img_canon)?;
		let mut loop_dev_fd = File::options()
			.write(true)
			.truncate(false)
			.append(false)
			.open(loopdev)?;
		let pos = loop_dev_fd.seek(std::io::SeekFrom::Start(offset))?;
		assert!(pos == offset);
		let mut bufrdr = BufReader::with_capacity(512, img_fd);
		copy(&mut bufrdr, &mut loop_dev_fd)?;
		Ok(())
	}

	fn apply_to_partition<P, Q, R>(img: P, container: Q, partition: R) -> Result<()>
	where
		P: AsRef<Path>,
		Q: AsRef<Path>,
		R: AsRef<Path>,
	{
		let img = img.as_ref();
		let container = container.as_ref();
		let partition = partition.as_ref();
		let img_canon = container.join(img.to_string_lossy().trim_start_matches('/'));
		let img_fd = File::options().read(true).create(false).open(&img_canon)?;
		let mut partition_fd = File::options()
			.write(true)
			.truncate(false)
			.append(false)
			.open(partition)?;
		let mut bufrdr = BufReader::with_capacity(512, img_fd);
		copy(&mut bufrdr, &mut partition_fd)?;
		Ok(())
	}
}

impl ImageContext<'_> {
	#[allow(unused_variables)]
	pub fn apply_bootloaders<P: AsRef<Path>>(&self, rootfs: P, loopdev: P) -> Result<()> {
		if self.device.bootloaders.is_none() {
			return Ok(());
		}
		self.info("Applying bootloaders ...");
		let rootfs = rootfs.as_ref();
		let loopdev = loopdev.as_ref();
		let bl_list = &self.device.bootloaders.as_ref().unwrap();
		for bl in *bl_list {
			match bl {
				BootloaderSpec::Script { name } => {
					BootloaderSpec::run_script(rootfs, Path::new(name))?;
				}
				BootloaderSpec::FlashPartition { path, partition } => {
					let partition = format!(
						"{}p{}",
						&loopdev.to_string_lossy(),
						partition
					);
					BootloaderSpec::apply_to_partition(
						path.as_path(),
						rootfs,
						Path::new(&partition),
					)?;
				}
				BootloaderSpec::FlashOffset { path, offset } => {
					BootloaderSpec::apply_offset(
						path, *offset, rootfs, loopdev,
					)?;
				}
			}
		}
		Ok(())
	}
}
