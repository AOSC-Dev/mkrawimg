use crate::{device::PartitionMapType, filesystem::FilesystemType};
use anyhow::{anyhow, Result};
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
#[allow(clippy::upper_case_acronyms)]
pub enum PartitionType {
	// Common types
	/// EFI System Partition
	/// - MBR: `0xef`
	/// - GPT: `C12A7328-F81F-11D2-BA4B-00A0C93EC93B`
	#[serde(alias = "esp")]
	EFI,
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
	#[serde(rename = "type", flatten)]
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
			Self::EFI => Ok(PARTTYPE_EFI_BYTE),
			Self::Linux => Ok(PARTTYPE_LINUX_BYTE),
			Self::Swap => Ok(PARTTYPE_SWAP_BYTE),
			Self::Basic => Ok(PARTTYPE_BASIC_BYTE),
			// Disallow extended partitions.
			Self::Byte { byte: 0x05 }
			| Self::Byte { byte: 0xc5 }
			| Self::Byte { byte: 0x85 }
			| Self::Byte { byte: 0x0f } => Err(anyhow!("Extended partitions are not allowed.")),
			Self::Byte { byte } => Ok(*byte),
			Self::Uuid { .. } => {
				Err(anyhow!("Can not convert an arbitrary byte to UUID."))
			}
			Self::Nested { .. } => {
				unimplemented!("Nested partition tables are not supported.")
			}
		}
	}
	pub fn to_uuid(&self) -> Result<Uuid> {
		match self {
			Self::EFI => Ok(PARTTYPE_EFI_UUID),
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
		assert_eq!(get!(TEST_EFI), Ok(PartitionType::EFI));
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
