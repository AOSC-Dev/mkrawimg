use std::{
	ffi::OsStr,
	fs,
	path::{Path, PathBuf},
};

use crate::{bootloader::BootloaderSpec, context::ImageVariant, partition::PartitionSpec};
use anyhow::{bail, Context, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PartitionMapType {
	MBR,
	GPT,
}

#[derive(
	Copy, Clone, Debug, strum::Display, Deserialize, PartialEq, Eq, PartialOrd, Ord, ValueEnum,
)]
#[serde(rename_all(deserialize = "snake_case"))]
pub enum DeviceArch {
	// Tier 1 architectures
	/// x86-64
	Amd64,
	/// AArch64
	Arm64,
	/// LoongArch64
	LoongArch64,
	// Tier 2 architectures
	/// IBM POWER 8 and up (Little Endian)
	Ppc64el,
	/// MIPS Loongson CPUs (Loongson 3, mips64el)
	Loongson3,
	/// 64-bit RISC-V with Extension C and G
	Riscv64,
	/// 64-Bit MIPS Release 6
	Mips64r6el,
}
/// Represents a device specification file device.toml.
#[derive(Clone, Debug, Deserialize)]
pub struct DeviceSpec {
	/// Unique ID of the device. Can be any ASCII characters except
	/// spaces, glob characters and /.
	pub id: String,
	/// Aliases to identify the exact device.
	pub aliases: Option<Vec<String>>,
	/// Vendor of the device.
	pub vendor: String,
	/// CPU Architecture of the device.
	pub arch: DeviceArch,
	/// Vendor of the SoC platform.
	/// The name must present in arch/$ARCH/boot/dts in the kernel tree.
	pub soc_vendor: String,
	/// Full name of the device for humans.
	pub name: String,
	/// Model name of the device, if it is different than the full name.
	pub model: Option<String>,
	/// The most relevant value of the compatible string in the root of the
	/// device tree, if it has one.
	///
	/// For example, the device tree file of Raspberry Pi 5B defines the following:
	/// ```dts
	/// / {
	/// 	compatible = "raspberrypi,5-model-b", "brcm,bcm2712";
	/// }
	/// ```
	/// We should choose `"raspberrypi,5-model-b"` for this.
	#[serde(rename = "compatible")]
	pub of_compatible: Option<String>,
	/// List of BSP packages to be installed.
	pub bsp_packages: Vec<String>,
	/// The partition map used for the image.
	pub partition_map: PartitionMapType,
	/// Number of the partitions.
	pub num_partitions: u32,
	/// Size of the image for each variant.
	pub size: ImageVariantSizes,
	/// Partitions in the image.
	// Can be `[[partition]]` to avoid awkwardness.
	#[serde(alias = "partition")]
	pub partitions: Vec<PartitionSpec>,
	/// Actions to apply bootloaders.
	#[serde(alias = "bootloader")]
	pub bootloaders: Option<Vec<BootloaderSpec>>,
	/// Path to the device.toml.
	#[serde(skip_deserializing)]
	pub file_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ImageVariantSizes {
	pub base: u64,
	pub desktop: u64,
	pub server: u64,
}

impl Default for ImageVariantSizes {
	fn default() -> Self {
		ImageVariantSizes {
			base: 5120,
			desktop: 25600,
			server: 6144,
		}
	}
}

impl DeviceSpec {
	pub fn from_path(file: &Path) -> Result<Self> {
		if file.file_name() != Some(OsStr::new("device.toml")) {
			bail!(
				"Filename in the path must be 'device.toml', got '{}'",
				file.display()
			)
		};
		let content = fs::read_to_string(file)
			.context(format!("Unable to read file '{}'", &file.to_string_lossy()))?;
		let mut device: DeviceSpec = toml::from_str(&content).context(format!(
			"Unable to treat '{}' as an entry of the registry",
			&file.to_string_lossy()
		))?;
		device.file_path = file.canonicalize()?;
		Ok(device)
	}
}

impl ImageVariantSizes {
	pub fn get_variant_size(&self, variant: &ImageVariant) -> u64 {
		match variant {
			ImageVariant::Base => self.base,
			ImageVariant::Desktop => self.desktop,
			ImageVariant::Server => self.server,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use log::info;
	use owo_colors::OwoColorize;

	#[test]
	fn test_from_path() -> Result<()> {
		env_logger::builder()
			.filter_level(log::LevelFilter::Debug)
			.build();
		let walker = walkdir::WalkDir::new("devices").max_depth(4).into_iter();
		for e in walker {
			let e = e?;
			if e.path().is_dir()
				|| e.path().file_name() != Some(OsStr::new("device.toml"))
			{
				continue;
			}
			info!("Parsing {} ...", e.path().display().bright_cyan());
			let device = DeviceSpec::from_path(e.path())?;
			info!("Parsed device:\n{:#?}", device);
		}
		Ok(())
	}
}
