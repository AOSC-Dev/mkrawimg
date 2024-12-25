use std::{
	collections::HashMap,
	ffi::OsStr,
	fs::{self, File},
	io::Write,
	path::{Path, PathBuf},
};

use crate::{
	bootloader::BootloaderSpec,
	context::{ImageContext, ImageVariant},
	partition::{PartitionSpec, PartitionType, PartitionUsage},
	pm::Distro,
};
use anyhow::{bail, Context, Result};
use clap::ValueEnum;
use gptman::{GPTPartitionEntry, GPT};
use log::debug;
use mbrman::{MBRPartitionEntry, CHS, MBR};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const FORBIDDEN_CHARS: &[char] = &['\'', '"', '\\', '/', '{', '}', '[', ']', '!', '`', '*', '&'];

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, strum::Display)]
#[serde(rename_all = "lowercase")]
// It is strange to see MBR as Mbr, GPT as Gpt.
#[allow(clippy::upper_case_acronyms)]
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
#[allow(dead_code)]
pub struct DeviceSpec {
	/// Unique ID of the device. Can be any ASCII characters except
	/// spaces, glob characters and /.
	pub id: String,
	/// Aliases to identify the exact device.
	pub aliases: Option<Vec<String>>,
	/// The distribution wich will be installed on this device.
	#[serde(default)]
	pub distro: Distro,
	/// Vendor of the device.
	pub vendor: String,
	/// CPU Architecture of the device.
	pub arch: DeviceArch,
	/// Vendor of the SoC platform.
	/// The name must present in arch/$ARCH/boot/dts in the kernel tree.
	pub soc_vendor: Option<String>,
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
	/// Whether the device boots without an initrd image.
	/// Useful for embedded systems (most of devices targeted by this
	/// project are embedded systems, aren't they).
	///
	/// If set to true, the following thing(s) will happen:
	/// - Generated fstab will use PARTUUID instead of filesystem UUID,
	///   since the kernel does not support using `UUID=` to specify root
	///   device if initrd is not being used.
	#[serde(default)]
	pub initrdless: bool,
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

#[allow(dead_code)]
pub struct PartitionMapData {
	pub uuid: String,
	/// Data for each partition
	pub data: HashMap<u32, PartitionData>,
}

#[derive(Clone)]
pub struct PartitionData {
	pub num: u32,
	pub part_uuid: String,
	pub fs_uuid: Option<String>,
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

	pub fn check(&self) -> Result<()> {
		let path: &Path = self.file_path.as_ref();
		let dirname = path.parent().context("Failed to get the directory containing the device spec file")?;
		let mut strs_to_chk = vec![&self.id, &self.vendor];
		if let Some(aliases) = &self.aliases {
			aliases.iter().for_each(|s| strs_to_chk.push(s));
		}
		if let Some(c) = &self.of_compatible {
			strs_to_chk.push(c)
		}
		for field in &strs_to_chk {
			if !field.is_ascii() {
				bail!("'{}' contains non-ASCII characters", field);
			}
			if field.contains(FORBIDDEN_CHARS) {
				bail!(
					"'{}' contains one of the following characters:\n{:?}",
					field,
					FORBIDDEN_CHARS
				);
			}
		}
		let mut strs_to_chk = vec![&self.name];
		if let Some(m) = &self.model {
			strs_to_chk.push(m);
		}
		for field in &strs_to_chk {
			if field.contains(FORBIDDEN_CHARS) {
				bail!(
					"'{}' contains one of the following characters:\n{:?}",
					field,
					FORBIDDEN_CHARS
				);
			}
		}
		if self.partitions.is_empty() {
			bail!("No partition defined for this device");
		}
		// Check consistency
		if self.num_partitions != self.partitions.len() as u32 {
			bail!(
				"Please update the num_partitions field: should be {}, got {}",
				self.partitions.len(),
				self.num_partitions
			);
		}
		// Can't have too many partitions
		let len = self.partitions.len();
		match self.partition_map {
			PartitionMapType::MBR => {
				if len > 4 {
					bail!("MBR partition map can contain up to 4 partitions");
				}
			}
			PartitionMapType::GPT => {
				if len > 128 {
					bail!("Too many partitions for GPT");
				}
			}
		}
		// Some devices may not have a boot partition.
		// Some devices may use MBR partition map.
		// Let's make the root partition the only requirement here.
		let mut root_part = None;
		let mut last_partition_num = 0;
		for partition in &self.partitions {
			if let Some(start) = partition.start_sector {
				if start <= 33 {
					bail!("Starting sector of partition {} overlaps the partition table itself.", partition.num);
				}
			}
			if partition.part_type == PartitionType::Swap {
				bail!("Swap partitions are not allowed on raw images.");
			}
			if partition.num == 0 {
				bail!("Partition numbers should start from 1.");
			}
			if partition.num < last_partition_num {
				bail!("Please keep the partitions in order");
			}
			if partition.num == last_partition_num {
				bail!("Duplicate partition number: {}", partition.num);
			}
			if partition.usage == PartitionUsage::Rootfs {
				if root_part.is_some() {
					bail!("More than one root partition defined");
				}
				root_part = Some(partition);
				if partition.mountpoint != Some("/".to_owned()) {
					bail!("Sorry, but for now root partition must have a mountpoint '/'.")
				}
			}
			if let Some(l) = &partition.label {
				if self.partition_map == PartitionMapType::MBR {
					bail!("MBR partition map does not allow partition labels, found one in partition {}", partition.num);
				}
				if l.len() > 35 {
					bail!("Label for partition {} exceeds the 35-character limit", partition.num);
				}
			}
			last_partition_num = partition.num;
			partition.filesystem.check(&partition.fs_label)?;
		}
		if root_part.is_none() {
			bail!("No root partition defined");
		}
		if let Some(bootloaders) = &self.bootloaders {
			for bl in bootloaders {
				match bl {
					BootloaderSpec::Script { name } => {
						let script_path = dirname.join(name);
						if !script_path.is_file() {
							bail!("Script '{}' not found within the same directory as the device.toml", &name);
						}
					}
					_ => (),
				}
			}
		}
		Ok(())
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

impl DeviceArch {
	pub fn get_native_arch() -> Option<&'static Self> {
		use std::env::consts::ARCH;
		match ARCH {
			"x86_64" => Some(&Self::Amd64),
			"aarch64" => Some(&Self::Arm64),
			"loongarch64" => Some(&Self::LoongArch64),
			"mips64" => {
				if cfg!(target_arch = "mips64r6") {
					Some(&Self::Mips64r6el)
				} else {
					Some(&Self::Loongson3)
				}
			}
			"riscv64" => Some(&Self::Riscv64),
			// TODO ppc64el needs work.
			"powerpc64" => Some(&Self::Ppc64el),
			_ => None,
		}
	}
	pub fn is_native(&self) -> bool {
		if let Some(a) = Self::get_native_arch() {
			if a == self {
				return true;
			}
		}
		false
	}

	pub fn get_qemu_binfmt_names(&self) -> &str {
		match self {
			Self::Amd64 => "qemu-x86_64",
			Self::Arm64 => "qemu-aarch64",
			Self::LoongArch64 => "qemu-loongarch64",
			Self::Ppc64el => "qemu-ppc64le",
			Self::Loongson3 => "qemu-mips64el",
			Self::Riscv64 => "qemu-riscv64",
			Self::Mips64r6el => "qemu-mips64el",
		}
	}
}

impl ImageContext<'_> {
	pub fn partition_gpt(&self, img: &Path) -> Result<PartitionMapData> {
		// The device must be opened write-only to write partition tables
		// Otherwise EBADF will be throwed
		let mut fd = File::options().write(true).open(img)?;
		// Use ioctl() to get sector size of the loop device
		// NOTE sector sizes can not be assumed
		let sector_size = gptman::linux::get_sector_size(&mut fd)?;
		debug!(
			"Got sector size of the loop device '{}': {} bytes",
			img.display(),
			sector_size
		);
		let rand_uuid = Uuid::new_v4();
		// NOTE UUIDs in GPT are like structs, they are "Mixed-endian."
		// The first three components are little-endian, and the last two are big-endian.
		// e.g. 01020304-0506-0708-090A-0B0C0D0E0F10 must be written as:
		//            LE       LE    LE
		//       vvvvvvvvvvv vvvvv vvvvv
		// 0000: 04 03 02 01 06 05 08 07
		// 0008: 09 0A 0B 0C 0D 0E 0F 10
		//       ^^^^^^^^^^^^^^^^^^^^^^^
		//              Big Endian
		// Uuid::to_bytes_le() produces the correct byte array.
		let disk_guid = rand_uuid.to_bytes_le();
		let mut new_table = GPT::new_from(&mut fd, sector_size, disk_guid)
			.context("Unable to create a new partition table")?;
		let mut parts_data: HashMap<u32, PartitionData> = HashMap::new();
		assert!(new_table.header.disk_guid == disk_guid);
		// 1MB aligned
		new_table.align = 1048576 / sector_size;
		self.info(format!(
			"Created new GPT partition table on {}:",
			img.display()
		));
		let size_in_lba = new_table.header.last_usable_lba;
		self.info(format!("UUID: {}", &rand_uuid));
		self.info(format!("Total LBA: {}", size_in_lba));
		let num_partitions = self.device.num_partitions;
		for partition in &self.device.partitions {
			if partition.num == 0 {
				bail!("Partition number must start from 1.");
			}
			let rand_part_uuid = Uuid::new_v4();
			let unique_partition_guid = rand_part_uuid.to_bytes_le();
			let free_blocks = new_table.find_free_sectors();
			debug!("Free blocks remaining: {:#?}", &free_blocks);
			let last_free = free_blocks
				.last()
				.context("No more free space available for new partitions")?;
			let size = if partition.size != 0 {
				partition.size
			} else {
				if partition.num != num_partitions {
					bail!("Max sized partition must stay at the end of the table.");
				}
				if last_free.1 < 1048576 / sector_size {
					bail!("Not enough free space to create a partition");
				}
				last_free.1 - 1
			};

			let partition_type_guid = partition.part_type.to_uuid()?.to_bytes_le();
			let starting_lba = if let Some(start) = partition.start_sector {
				start
			} else if partition.num == 1 {
				// 1MB grain size to reserve some space for bootloaders
				1048576 / sector_size as u64
			} else {
				new_table.find_first_place(size).context(format!(
					"No suitable space found for partition:\n{:?}.",
					&partition
				))?
			};
			let ending_lba = starting_lba + size - 1;
			let name = if let Some(name) = partition.label.to_owned() {
				name
			} else {
				"".into()
			};
			let partition_name = name.as_str();
			self.info(format!(
				"Creating an {:?} partition with PARTUUID {}:",
				partition.part_type, rand_part_uuid
			));
			self.info(format!(
				"Size in LBA: {}, Start = {}, End = {}",
				size, starting_lba, ending_lba
			));
			let part = GPTPartitionEntry {
				partition_type_guid,
				unique_partition_guid,
				starting_lba,
				ending_lba,
				attribute_bits: 0,
				partition_name: partition_name.into(),
			};
			new_table[partition.num] = part;
			parts_data.insert(
				partition.num,
				PartitionData {
					num: partition.num,
					part_uuid: rand_part_uuid.to_string(),
					fs_uuid: None,
				},
			);
		}
		self.info("Writing changes ...");
		// Protective MBR is written for compatibility.
		// Plus, most partitioning program will not accept pure GPT
		// configuration, they will warn about missing Protective MBR.
		GPT::write_protective_mbr_into(&mut fd, sector_size)?;
		new_table.write_into(&mut fd)?;
		fd.sync_all()?;
		let pm_data = PartitionMapData {
			uuid: rand_uuid.to_string(),
			data: parts_data,
		};
		Ok(pm_data)
	}

	pub fn partition_mbr(&self, img: &Path) -> Result<PartitionMapData> {
		let mut fd = File::options().write(true).open(img)?;
		let sector_size =
			TryInto::<u32>::try_into(gptman::linux::get_sector_size(&mut fd)?)
				.unwrap_or(512);
		let random_id: u32 = rand::random();
		let disk_signature = random_id.to_le_bytes();
		let disk_signature_str = format!("{:08x}", random_id);
		let mut new_table = MBR::new_from(&mut fd, sector_size, disk_signature)?;
		let mut parts_data: HashMap<u32, PartitionData> = HashMap::new();
		self.info(format!("Created a MBR table on {}:", img.display()));
		// Human readable format
		self.info(format!(
			"Disk signature: {:X}-{:X}",
			(random_id >> 16) as u16,
			(random_id & 0xffff) as u16
		));
		for partition in &self.device.partitions {
			if partition.num == 0 {
				bail!("Partition number must start from 1.");
			}
			if partition.num > 4 {
				bail!("Extended and logical partitions are not supported.");
			}
			let free_blocks = new_table.find_free_sectors();
			debug!("Free blocks remaining: {:#?}", &free_blocks);
			let last_free = free_blocks
				.last()
				.context("No more free space available for new partitions")?;
			let idx = TryInto::<usize>::try_into(partition.num)
				.context("Partition number exceeds the limit")?;
			let sectors = if partition.size != 0 {
				TryInto::<u32>::try_into(partition.size)
					.context("Partition size exceeds the limit of MBR")?
			} else {
				// Make sure it is the last partition.
				if partition.num != self.device.num_partitions {
					bail!("Max sized partition must stay at the end of the table.");
				}
				last_free.1 - 1
			};
			if sectors < 1048576 / sector_size {
				bail!("Not enough free space to create a partition");
			}
			let starting_lba = if let Some(start) = partition.start_sector {
				TryInto::<u32>::try_into(start)
					.context("Partition size exceeds the limit of MBR")?
			} else if partition.num == 1 {
				// 1MB grain size to reserve some space for bootloaders
				1048576 / sector_size as u32
			} else {
				new_table.find_first_place(sectors).context(format!(
					"No suitable free space found for partition: {:?}",
					&partition
				))?
			};
			let boot = if partition.usage == PartitionUsage::Boot {
				mbrman::BOOT_ACTIVE
			} else {
				mbrman::BOOT_INACTIVE
			};
			let sys = partition.part_type.to_byte()?;
			self.info(format!("Creating an {:?} partition:", &partition.part_type));
			self.info(format!(
				"Size in LBA: {}, Start = {}, End = {}",
				sectors,
				starting_lba,
				starting_lba + sectors - 1
			));
			let part = MBRPartitionEntry {
				boot,
				first_chs: CHS::empty(),
				sys,
				last_chs: CHS::empty(),
				starting_lba,
				sectors,
			};
			new_table[idx] = part;
			parts_data.insert(
				partition.num,
				PartitionData {
					num: partition.num,
					part_uuid: format!("{}-{:02x}", &disk_signature_str, idx),
					fs_uuid: None,
				},
			);
		}
		self.info("Writing the partition table ...");
		new_table.write_into(&mut fd)?;
		fd.sync_all()?;
		let pm_data = PartitionMapData {
			uuid: disk_signature_str,
			data: parts_data,
		};
		Ok(pm_data)
	}

	pub fn write_spec_script(
		&self,
		loopdev: &dyn AsRef<Path>,
		rootpart: &dyn AsRef<Path>,
		container: &dyn AsRef<Path>,
		pm_data: &PartitionMapData,
	) -> Result<()> {
		let mut script = format!(
			r#"DEVICE_ID='{0}'
DEVICE_COMPATIBLE='{1}'
LOOPDEV='{2}'
NUM_PARTITIONS='{3}'
ROOTPART='{4}'
DISKLABEL='{5}'
DISKUUID='{6}'
"#,
			self.device.id,
			&self.device.of_compatible.clone().unwrap_or("".to_string()),
			loopdev.as_ref().to_string_lossy(),
			self.device.num_partitions,
			rootpart.as_ref().to_string_lossy(),
			&self.device.partition_map.to_string().to_lowercase(),
			&pm_data.uuid,
		);
		for part in &self.device.partitions {
			let part_data = pm_data.data.get(&part.num).context(format!(
				"Unable to get partition data for partition {}",
				part.num
			))?;
			assert_eq!(part.num, part_data.num);
			script += &format!(
				"PART{0}_PARTUUID='{1}'\n",
				part_data.num, part_data.part_uuid,
			);
			if part.usage == PartitionUsage::Rootfs {
				script +=
					&format!("ROOT_PARTUUID=\"$PART{0}_PARTUUID\"\n", part.num);
			} else if part.usage == PartitionUsage::Boot {
				script +=
					&format!("BOOT_PARTUUID=\"$PART{0}_PARTUUID\"\n", part.num);
			}
			if part.part_type == PartitionType::EFI {
				script +=
					&format!("EFI_PARTUUID=\"$PART{0}_PARTUUID\"\n", part.num);
			}
			// We might not have a filesystem UUID under some circumstances
			if let Some(fsuuid) = &part_data.fs_uuid {
				script +=
					&format!("PART{0}_FSUUID='{1}'\n", part_data.num, &fsuuid);
				if part.usage == PartitionUsage::Rootfs {
					script += &format!(
						"ROOT_FSUUID=\"$PART{0}_FSUUID\"\n",
						part.num
					);
				} else if part.usage == PartitionUsage::Boot {
					script += &format!(
						"BOOT_FSUUID=\"$PART{0}_FSUUID\"\n",
						part.num
					);
				}
				if part.part_type == PartitionType::EFI {
					script += &format!(
						"EFI_FSUUID=\"$PART{0}_FSUUID\"\n",
						part.num
					);
				}
			}
		}
		debug!("Script content: \n{}", &script);
		let path = container.as_ref().join("tmp/spec.sh");
		let mut fd = File::options()
			.create(true)
			.write(true)
			.truncate(true)
			.open(&path)?;
		fd.write_all(script.as_bytes())?;
		fd.flush()?;
		fd.sync_all()?;
		Ok(())
	}

	pub fn generate_fstab(
		&self,
		pm_data: &PartitionMapData,
		container: &dyn AsRef<Path>,
	) -> Result<()> {
		self.info("Generating /etc/fstab ...");
		let mut content = String::from("\n# ---- Auto generated by mkrawimg ----\n");
		for partition in &self.device.partitions {
			if let Some(mountpoint) = &partition.mountpoint {
				let part_data =
					pm_data.data.get(&partition.num).context(format!(
						"Unable to get partition data for partition {}",
						partition.num
					))?;
				let src = if self.device.initrdless {
					format!("PARTUUID=\"{0}\"", &part_data.part_uuid)
				} else {
					format!("UUID=\"{0}\"", &part_data.fs_uuid.as_ref().context("Partition with a mountpoint must have a valid filesystem")?)
				};
				// dst = mountpoint
				// `genfstab(8)` uses the options field in `/proc/mounts`, which is the expanded result from `defaults`.
				let options = "defaults";
				let fsck_passno = if partition.usage == PartitionUsage::Rootfs {
					1
				} else {
					2
				};
				let entry = format!(
					"{0}\t{1}\t{2}\t{3}\t{4}\t{5}\n",
					&src,
					&mountpoint,
					&partition.filesystem.get_os_fstype()?,
					options,
					0,
					fsck_passno
				);
				content += &entry;
			} else {
				// We can not generate fstab entry for partitions without a mountpoint
				continue;
			}
		}
		let fstab_path = container.as_ref().join("etc/fstab");
		let mut fstab_fd = File::options()
			.truncate(false)
			.append(true)
			.open(&fstab_path)?;
		fstab_fd.write_all(content.as_bytes())?;
		fstab_fd.flush()?;
		fstab_fd.sync_all()?;
		Ok(())
	}

	pub fn set_hostname(&self, container: &dyn AsRef<Path>) -> Result<()> {
		self.info("Setting up hostname ...");
		let rand_id: u32 = rand::random();
		let hostname = format!(
			"{:?}-{}-{:08x}",
			&self.device.distro, &self.device.id, rand_id
		);
		self.info(format!("Hostname: {}", &hostname));
		let hostname_path = container.as_ref().join("etc/hostname");
		let mut hostname_fd = File::options()
			.truncate(true)
			.write(true)
			.create(true)
			.open(hostname_path)?;
		hostname_fd.write_all(hostname.as_bytes())?;
		hostname_fd.flush()?;
		hostname_fd.sync_all()?;
		let hosts_entries = format!("\n127.0.0.1\t{0}\n::1\t{0}\n", hostname);
		let hosts_fd = container.as_ref().join("etc/hosts");
		let mut hosts_fd = File::options().append(true).create(true).open(hosts_fd)?;
		hosts_fd.write_all(hosts_entries.as_bytes())?;
		hosts_fd.flush()?;
		hosts_fd.sync_all()?;
		Ok(())
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
