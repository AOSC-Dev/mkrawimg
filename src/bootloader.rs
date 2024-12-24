//! Module handling the application of the bootloader images.
//!
//! You can perform one or more of the following actions to apply bootloaders:
//!
//! - Run a script (within the same directory as the `device.toml` file)
//! - Apply (“flash”) a file to the specific partition of the target image
//! - Apply (“flash”) a file to the specific offset of the target image
//!
//! For details please go to [`BootloaderSpec`].
//!
use std::{
	fs::File,
	io::{copy, BufReader, Seek},
	path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use log::info;
use serde::Deserialize;

use crate::{context::ImageContext, utils::run_script_with_chroot};

/// Specifies how to apply a bootloader image (file) to the target image.
///
/// You can write a file (inside the target filesystem) to a specific partition, or to a specific location (offset) of the target image,
/// or use a script to finish this step.
///
/// Multiple entries are allowed, thus you can flash multiple files and run different scripts. The list will be executed sequencially.
///
/// Examples
/// --------
///
/// In your `device.toml`, add one or more of the `[[bootloader]]` list items. Examples of `[[bootloader]]` entries are shown below:
///
/// ### Run a script within the same directory as `device.toml`
///
/// ```toml
/// [[bootloader]]
/// type = script
/// # Path to the script file.
/// # This file must reside in the same directory as the device.toml file.
/// name = apply-bootloader.sh
/// ```
/// ### Flash a bootloader image to the specific partition of the target image
///
/// ```toml
/// [[bootloader]]
/// type = flash_partition
/// # Path to the bootloader image within the target root filesystem (symbolic links allowed).
/// path = "/usr/lib/u-boot/rk64/rk3588-orange-pi-5-max-idbloader.img"
/// # The index of the target partition
/// partition = 1
/// ```
///
/// ```toml
/// [[bootloader]]
/// type = flash_partition
/// # Path to the bootloader image within the target root filesystem (symbolic links allowed).
/// path = "/usr/lib/u-boot/rk64/rk3588-orange-pi-5-max.itb"
/// partition = 2
/// ```
///
/// ### Flash a bootloader image to the specific location of the target image
///
/// ```toml
/// [[bootloader]]
/// type = flash_offset
/// path = "/path/to/bootlodaer/image"
/// # Offset from the start of the target image in bytes.
/// offset = 0x400
/// ```
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum BootloaderSpec {
	/// Run the script within the same directory as `device.toml`.
	///
	/// The script will be copied to a temporary location in the target filesystem.
	/// The script has access to all of the exported partition information.
	///
	/// ```toml
	/// [[bootloader]]
	/// type = script
	/// # Path to the script file.
	/// # This file must reside in the same directory as the device.toml file.
	/// name = apply-bootloader.sh
	/// ```
	Script { name: String },
	/// Flash a bootloader image to the specific partition of the target image.
	///
	/// The path must be (or point to) a regular file within the target root filesystem.
	///
	/// ```toml
	/// [[bootloader]]
	/// type = flash_partition
	/// # Path to the bootloader image within the target root filesystem (symbolic links allowed).
	/// path = "/usr/lib/u-boot/rk64/rk3588-orange-pi-5-max-idbloader.img"
	/// # The index of the target partition
	/// partition = 1
	/// ```
	FlashPartition { path: PathBuf, partition: u64 },
	/// Flash a bootloader image to the specific location of the target image.
	///
	/// The path must be (or point to) a regular file within the target root filesystem.
	///
	/// <div class="warning">
	///
	/// - Always make sure the image will not overlap existing partitions and filesystems.
	/// - If your bootloader image is too large (e.g. exceeds 960KiB), you must adjust the starting position of the first partition (since the default starting sector is 2048 (1 MiB)).
	/// - Therefore it is advised to create dedicated partitions reserved for bootloaders and flash them to their specific partition.
	///
	/// </div>
	///
	/// ```toml
	/// [[bootloader]]
	/// type = flash_offset
	/// path = "/path/to/bootlodaer/image"
	/// # Offset from the start of the target image in bytes.
	/// offset = 0x400
	/// ```
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
		info!("Running script {}", script.display());
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
		let device_spec_dir =
			self.device.file_path.parent().context(
				"Failed to reach the directory containing the device spec file",
			)?;
		for bl in *bl_list {
			match bl {
				BootloaderSpec::Script { name } => {
					BootloaderSpec::run_script(
						rootfs,
						device_spec_dir.join(name),
					)?;
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
