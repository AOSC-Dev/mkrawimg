use std::{fs::File, path::Path};

use crate::{context::ImageContext, device::PartitionMapType, filesystem::FilesystemType};
use anyhow::{anyhow, bail, Context, Result};
use gptman::{GPTPartitionEntry, GPT};
use log::debug;
use mbrman::{MBRPartitionEntry, CHS, MBR};
use serde::{Deserialize, Serialize};
use uuid::{uuid, Uuid};

pub const PARTTYPE_EFI_UUID: Uuid = uuid!("C12A7328-F81F-11D2-BA4B-00A0C93EC93B");
pub const PARTTYPE_LINUX_UUID: Uuid = uuid!("0FC63DAF-8483-4772-8E79-3D69D8477DE4");
pub const PARTTYPE_SWAP_UUID: Uuid = uuid!("0657FD6D-A4AB-43C4-84E5-0933C84B4F4F");
pub const PARTTYPE_BASIC_UUID: Uuid = uuid!("EBD0A0A2-B9E5-4433-87C0-68B6B72699C7");

pub const PARTTYPE_EFI_BYTE: u8 = 0xEF;
pub const PARTTYPE_LINUX_BYTE: u8 = 0x83;
pub const PARTTYPE_SWAP_BYTE: u8 = 0x82;
pub const PARTTYPE_BASIC_BYTE: u8 = 0x07;

#[derive(Deserialize, Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]

pub enum PartitionType {
	// Common types
	/// EFI System Partition
	/// - MBR: `0xef`
	/// - GPT: `C12A7328-F81F-11D2-BA4B-00A0C93EC93B`
	#[serde(alias = "esp")]
	Efi,
	/// Linux filesystem data
	/// - MBR: `0x83`
	/// - GPT: `0FC63DAF-8483-4772-8E79-3D69D8477DE4`
	Linux,
	/// Swap partition
	/// MBR - `0x82`
	/// GPT - `0657FD6D-A4AB-43C4-84E5-0933C84B4F4F`
	Swap,
	/// Basic Data Partition
	/// MBR - `0x07`
	/// GPT - `EBD0A0A2-B9E5-4433-87C0-68B6B72699C7`
	Basic,
	/// Arbitary UUID values.
	Uuid {
		/// Arbitary UUID can be specified here.
		uuid: Uuid,
	},
	/// Arbitary MBR partition types.
	Byte {
		/// Arbitary MBR partition type can be specified here.
		byte: u8,
	},
	/// Nested partition table. Will not implemented, so being here is just for fun.
	/// Who the hell in this world wants to use this anyway?
	Nested {
		table_type: PartitionMapType,
		partitions: Vec<PartitionSpec>,
	},
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PartitionSpec {
	#[serde(alias = "no")]
	pub num: u32,
	#[serde(rename = "type")]
	pub part_type: PartitionType,
	pub start_sector: Option<u64>,
	pub size: u64,
	pub label: Option<String>,
	pub mountpoint: Option<String>,
	pub filesystem: FilesystemType,
	pub fs_label: Option<String>,
	pub usage: PartitionUsage,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PartitionUsage {
	Boot,
	Rootfs,
	Swap,
	Data,
	Other,
}

impl PartitionType {
	pub fn to_byte(&self) -> Result<u8> {
		match self {
			Self::Efi => Ok(PARTTYPE_EFI_BYTE),
			Self::Linux => Ok(PARTTYPE_LINUX_BYTE),
			Self::Swap => Ok(PARTTYPE_SWAP_BYTE),
			Self::Basic => Ok(PARTTYPE_BASIC_BYTE),
			// Disallow extended partitions.
			Self::Byte { byte: 0x05 }
			| Self::Byte { byte: 0xc5 }
			| Self::Byte { byte: 0x85 }
			| Self::Byte { byte: 0x0f } => Err(anyhow!("Extended partitions are not allowed.")),
			Self::Byte { byte } => Ok(*byte),
			Self::Uuid { uuid } => {
				Err(anyhow!("Can not convert an arbitrary byte to UUID."))
			}
			Self::Nested { .. } => {
				unimplemented!("Nested partition tables are not supported.")
			}
		}
	}
	pub fn to_uuid(&self) -> Result<Uuid> {
		match self {
			Self::Efi => Ok(PARTTYPE_EFI_UUID),
			Self::Linux => Ok(PARTTYPE_LINUX_UUID),
			Self::Swap => Ok(PARTTYPE_SWAP_UUID),
			Self::Basic => Ok(PARTTYPE_BASIC_UUID),
			Self::Uuid { uuid } => Ok(*uuid),
			Self::Byte { .. } => Err(anyhow!("Can not convert an MBR type to UUID.")),
			Self::Nested { .. } => {
				Err(anyhow!("Nested partition tables are not supported."))
			}
		}
	}
}

impl ImageContext<'_> {
	pub fn partition_gpt(&self, img: &Path) -> Result<()> {
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
		//            LE          LE
		//       vvvvvvvvvvv vvvvvvvvvvv
		// 0000: 04 03 02 01 08 07 06 05
		// 0008: 09 0A 0B 0C 0D 0E 0F 10
		//       ^^^^^^^^^^^^^^^^^^^^^^^
		//              Big Endian
		// Uuid::to_bytes_le() produces the correct byte array.
		let disk_guid = rand_uuid.to_bytes_le();
		let mut new_table = gptman::GPT::new_from(&mut fd, sector_size, disk_guid)
			.context("Unable to create a new partition table")?;
		assert!(new_table.header.disk_guid == disk_guid);
		// 1MB aligned
		new_table.align = 1048576 / sector_size;
		self.info(format!(
			"Created new GPT partition table on {}:",
			img.display()
		));
		let size_in_lba = new_table.header.last_usable_lba;
		self.info(format!("UUID: {}", rand_uuid));
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
		}
		self.info("Writing changes ...");
		// Protective MBR is written for compatibility.
		// Plus, most partitioning program will not accept pure GPT
		// configuration, they will warn about missing Protective MBR.
		GPT::write_protective_mbr_into(&mut fd, sector_size)?;
		new_table.write_into(&mut fd)?;
		fd.sync_all()?;
		Ok(())
	}

	pub fn partition_mbr(&self, img: &Path) -> Result<()> {
		let mut fd = File::options().write(true).open(img)?;
		let sector_size =
			TryInto::<u32>::try_into(gptman::linux::get_sector_size(&mut fd)?)
				.unwrap_or(512);
		let random_id: u32 = rand::random();
		let disk_signature = random_id.to_be_bytes();
		let mut new_table = MBR::new_from(&mut fd, sector_size, disk_signature)?;
		self.info(format!("Created a MBR table on {}:", img.display()));
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
				return Err(anyhow!(
					"Extended and logical partitions are not supported."
				));
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
		}
		self.info("Writing the partition table ...");
		new_table.write_into(&mut fd)?;
		fd.sync_all()?;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	const TEST_EFI: &str = r#"type = "efi""#;
	const TEST_LINUX: &str = r#"type = "linux""#;
	const TEST_SWAP: &str = r#"type = "swap""#;
	const TEST_BASIC: &str = r#"type = "basic""#;
	const TEST_INVALID: &str = r#"type = "whatever""#;
	const TEST_UUID: &str = "type = \"uuid\"\nuuid = \"933AC7E1-2EB4-4F13-B844-0E14E2AEF915\"";
	const TEST_BYTE: &str = "type = \"byte\"\nbyte = 0x0c";
	const TEST_EXTENDED: &str = "type = \"byte\"\nbyte = 0x05";

	use super::*;
	use toml;
	macro_rules! get {
		($x:ident) => {
			toml::from_str::<PartitionType>($x)
		};
	}
	#[test]
	fn test_part_type() -> Result<()> {
		let x = get!(TEST_EFI);
		assert_eq!(get!(TEST_EFI), Ok(PartitionType::Efi));
		assert_eq!(get!(TEST_LINUX), Ok(PartitionType::Linux));
		assert_eq!(get!(TEST_SWAP), Ok(PartitionType::Swap));
		assert_eq!(get!(TEST_BASIC), Ok(PartitionType::Basic));
		assert_eq!(
			get!(TEST_UUID),
			Ok(PartitionType::Uuid {
				uuid: uuid!("933AC7E1-2EB4-4F13-B844-0E14E2AEF915")
			})
		);
		assert_eq!(get!(TEST_BYTE), Ok(PartitionType::Byte { byte: 0x0c }));
		assert_eq!(
			get!(TEST_EXTENDED)
				.unwrap()
				.to_byte()
				.unwrap_err()
				.to_string(),
			String::from("Extended partitions are not allowed.")
		);
		Ok(())
	}
}
