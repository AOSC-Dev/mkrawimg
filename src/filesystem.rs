use anyhow::{anyhow, bail, Ok, Result};
use serde::{Deserialize, Serialize};
use std::{path::Path, process::Command};

use crate::context::ImageContext;

/// Speifies which filesystem to be formatted to a partition.
#[derive(Copy, Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FilesystemType {
	/// Linux extended filesystem version 4.
	Ext4,
	/// XFS from Sun Microsystems.
	Xfs,
	/// B-tree filesystem.
	Btrfs,
	/// FAT16.
	Fat16,
	/// FAT32.
	Fat32,
	/// Not to be formatted.
	Null,
}

impl FilesystemType {
	/// Check validaty of the filesystem parameters.
	pub fn check<S: AsRef<str>>(&self, label: Option<S>) -> Result<()> {
		let label = label.as_ref();
		// Check for validity of the filesystem labels.
		if let Some(l) = label {
			let l = l.as_ref();
			match self {
				Self::Fat16 | Self::Fat32 => {
					if !l.is_ascii() {
						bail!("FAT volume label can only contain ASCII characters.");
					}
					if l.len() > 11 {
						bail!("FAT Volume labels can not be longer than 11 characters.");
					}
				}
				_ => {
					if l.as_bytes().len() > 63 {
						bail!("Filesystem labels are limited to up to 63 bytes.");
					}
				}
			};
		}
		Ok(())
	}

	pub fn get_os_fstype(&self) -> Result<&'static str> {
		match self {
			FilesystemType::Ext4 => Ok("ext4"),
			FilesystemType::Xfs => Ok("xfs"),
			FilesystemType::Btrfs => Ok("btrfs"),
			FilesystemType::Fat16 | FilesystemType::Fat32 => Ok("vfat"),
			FilesystemType::Null => {
				Err(anyhow!("It is instructed to not being formatted"))
			}
		}
	}

	pub fn get_mkfs_cmdline(
		&self,
		path: &dyn AsRef<Path>,
		label: Option<String>,
	) -> Result<Command> {
		if self == &Self::Null {
			bail!("Instructed to not being formatted");
		}
		let path = path.as_ref();
		self.check(label.to_owned())?;
		// Decide which command to use.
		let mut mkfs_command = Command::new(match self {
			Self::Ext4 => "mkfs.ext4",
			Self::Btrfs => "mkfs.btrfs",
			Self::Xfs => "mkfs.xfs",
			Self::Fat16 | Self::Fat32 => "mkfs.vfat",
			_ => {
				unreachable!();
			}
		});

		if let Some(l) = label {
			mkfs_command.arg(match self {
				Self::Ext4 => "-L",
				Self::Xfs => "-L",
				Self::Btrfs => "-L",
				Self::Fat16 | Self::Fat32 => "-n",
				_ => {
					unreachable!()
				}
			});
			mkfs_command.arg(l);
		}
		mkfs_command.arg("--");
		mkfs_command.arg(path);
		Ok(mkfs_command)
	}

	pub fn format(&self, path: &dyn AsRef<Path>, label: Option<String>) -> Result<()> {
		let mut cmd = self.get_mkfs_cmdline(path, label)?;
		cmd_run_check_status(&mut cmd)
	}
}

impl ImageContext<'_> {
	pub fn format_partitions(&self, loopdev: &dyn AsRef<Path>) -> Result<()> {
		let loopdev = loopdev.as_ref();
		for partition in &self.device.partitions {
			if partition.filesystem == FilesystemType::Null {
				continue;
			}
			let filesystem = if partition.usage == PartitionUsage::Rootfs {
				if let Some(fstype) = self.override_rootfs_fstype {
					fstype
				} else {
					&partition.filesystem
				}
			} else {
				&partition.filesystem
			};
			self.info(format!(
				"Formatting partition {} ({:?})",
				partition.num, filesystem
			));
			let num = partition.num;
			let part_path = format!("{}p{}", loopdev.to_string_lossy(), num);
			let label = &partition.label;
			filesystem.format(&part_path, label.to_owned())?;
		}
		Ok(())
	}
}
