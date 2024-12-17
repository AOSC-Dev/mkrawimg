//! `mod bootloader`
//! This module implements simple logic to apply bootloader images to the target raw media image.
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;

use crate::context::ImageContext;

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
/// ```
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum BootloaderSpec {
	/// Run the script within the same directory as `device.toml`
	Script { name: String },
	/// Flash a file (inside the target image) to a specific partition
	FlashPartition { path: PathBuf, partition: usize },
	/// Flash a file (inside the target image) to a specfifc offset at the image
	FlashOffset { path: PathBuf, offset: usize },
}

impl ImageContext<'_> {
	pub fn apply_bootloaders<P: AsRef<Path>>(&self, rootfs: P, loopdev: P) -> Result<()> {
		Ok(())
	}
}
