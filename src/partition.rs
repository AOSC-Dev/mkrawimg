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
/// Partition type recorded in the partition table.
///
/// For MBR partition table, it is a defined single byte value.
/// For GPT partition table, it is a defiined GUID.
///
/// ```toml
/// [[partition]]
/// # other fields
/// type = "efi"
/// ```
pub enum PartitionType {
	// Common types
	/// EFI System Partition
	/// - MBR: `0xef`
	/// - GPT: `C12A7328-F81F-11D2-BA4B-00A0C93EC93B`
	///
	/// ```toml
	/// # other fields
	/// type = "esp"
	/// # or
	/// type = "efi"
	/// ```
	#[serde(alias = "esp")]
	EFI,
	/// Linux filesystem data
	/// - MBR: `0x83`
	/// - GPT: `0FC63DAF-8483-4772-8E79-3D69D8477DE4`
	///
	/// ```toml
	/// # other fields
	/// type = "linux"
	/// ```
	Linux,
	/// Swap partition
	///
	/// <div class="warning">
	/// Swap partitions are not allowed due to various reasons. The tool will throw an error if a swap partition is encountered.
	/// </div>
	///
	/// - MBR: `0x82`
	/// - GPT: `0657FD6D-A4AB-43C4-84E5-0933C84B4F4F`
	///
	/// ```toml
	/// # other fields
	/// type = "swap"
	/// ```
	Swap,
	/// Basic Data Partition
	/// - MBR: `0x07`
	/// - GPT: `EBD0A0A2-B9E5-4433-87C0-68B6B72699C7`
	///
	/// ```toml
	/// [[partition]]
	/// # other fields
	/// type = "basic"
	/// ```
	Basic,
	/// Arbitary UUID values.
	///
	/// If being used on a MBR partition table, the program will throw an error.
	///
	/// ```toml
	/// [[partition]]
	/// # other fields
	/// type = "uuid"
	/// uuid = "01234567-89AB-CDEF-0123-456789ABCDEF"
	/// ```
	Uuid {
		/// Arbitary UUID can be specified here.
		uuid: Uuid,
	},
	/// Arbitary MBR partition types.
	///
	/// If being used on a GPT partition table, the program will throw an error.
	///
	/// ```toml
	/// [[partition]]
	/// type = "byte"
	/// byte = 0xef
	/// ```
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

/// Partition specification
/// =======================
///
/// Describes partitions in the partition table, and is defined in the [Device Specification File]. It is a list of objects in the TOML format.
///
/// It describes various aspects of a partition:
///
/// - The partition type recorded in the partition table.
/// - The size of the partition.
/// - Where the partition starts at, in 512-byte sectors.
/// - Whether it contains a filesystem.
/// - Whether it has a mountpoint.
/// - The usage of the partition, i.e. being used as a boot partition, or as a root partition.
/// - Whether the filesystem has labels.
/// - For GPT partition table, an optional partition label can be defined.
///
/// Fields
/// ======
///
/// `num` - Partition number
/// ------------------------
///
/// Index of the partition in the partition table.
///
/// ```toml
/// [[partition]]
/// num = 1
/// ```
///
/// `type` - Partition Type
/// -----------------------
///
/// Partition type defined in the partition table. Not to be confused by filesystem type.
///
/// Possible values are:
///
/// - [`"efi"`]: EFI System Partition.
/// - [`"linux"`]: Linux filesystem.
/// - [`"swap"`]: Swap partition (not allowed to be used).
/// - [`"basic"`]: Basic data partition.
/// - [`"uuid"`]: Arbitrary UUID value. An additional field `uuid` is required to specify the UUID value.
/// - [`"byte"`]: Arbitrary byte value. An additional field `byte` is required to specify the byte value.
///
/// ```toml
/// [[partition]]
/// # other fields
/// type = "efi"
/// # or
/// type = "linux"
/// # or
/// type = "swap"
/// # or
/// type = "basic"
/// # Or an arbitrary UUID value
/// type = "uuid"
/// uuid = "01234567-89AB-CDEF-0123-456789ABCDEF"
/// # Or an arbitrary byte value
/// type = "byte"
/// byte = 0x0c
/// ```
///
/// `start_sector` - Starting position (Optional)
/// ---------------------------------------------
///
/// Defines where the partition starts in the partition table, in 512-byte sectors.
///
/// If not defined, then this partition will immidiately follow the previous partition, or starts at sector `2048`` if this is the first partition, leaving ~1MB empty space before it.
///
/// For example, your device requires a bootloader partition to be present at 32KB from start, then the value would be:
///
/// ```toml
/// # other fields
/// start_sector = 64
/// ```
///
/// `size_in_sectors` - Partition size
/// -----------------------
///
/// Defines the size of the partition, in 512-byte sectors.
///
/// Use `0` if you want to fill the partition all the way to the end - only for the last partition.
///
/// For example, for a 300MiB partition, the value would be `300 * 1024 * 2 = 614400` (1 KiB = 2 sectors).
///
/// A partition can not be smaller than 512B (1 sector).
///
/// ```toml
/// [[partition]]
/// # other fields ...
/// # 1MiB partition
/// size = 2048
/// # 1GiB partition
/// size = 2097152
/// # Max available free space
/// size = 0
/// ```
///
/// `label` - Partition label (GPT Only, Optional)
/// ----------------------------------------------
///
/// Name of the partition, only available on GPT partition table. **Not to be confused by filesystem labels.**
///
/// The partition label has a 64-characters limit.
///
/// ```toml
/// [[partition]]
/// # other fields ...
/// label = "EFI Partition"
/// ```
///
/// `filesystem` - Filesystem contained in the partition
/// ----------------------------------------------------
///
/// The filesystem to be created on the partition.
///
/// Possible values are:
///
/// - `ext4`: Linux Extended filesystem version 4.
/// - `btrfs`: B-Tree filesystem.
/// - `xfs`: XFS from Sun Microsystems.
/// - `fat16`: FAT16 filesystem, can not be used as the root filesystem.
/// - `fat32`: FAT32 filesystem, can not be used as the root filesystem.
/// - `none`: Not to be formatted.
///
/// ```toml
/// [[partition]]
/// # other fields ...
/// filesystem = "ext4"
/// ```
///
/// `fs_label` - Filesystem label (Optional)
/// ----------------------------------------
///
/// Label of the filesystem. Its limitation varies by filesystem type.
///
/// ```toml
/// [[partition]]
/// # other fields ...
/// fs_label = "AOSC OS"
/// ```
///
/// `mountpoint` - Mount point of the filesystem
/// --------------------------------------------
///
/// Where the filesystem should be mounted in the target OS. A partition without a filesystem can not be mounted.
///
/// The root filesystem must have a mountpoint "/".
///
/// ```toml
/// [[partition]]
/// # other fields ...
/// mountpoint = "/boot/firmware"
/// ```
///
/// `mount_opts` - Mount options (Optional)
/// ---------------------------------------
///
/// Mount options used to mount the filesystem. This will be present in the generated `/etc/fstab`.
///
/// <div class="warning">
/// The handling of the mount options is not complete, only options specific to the filesystem type are allowed.
///
/// That is, options like <code>ro</code>, <code>noexec</code> and <code>nosuid</code> are not handled, and will result in an error if specified.
/// </div>
///
/// If not defined, `defaults` will be used. If defined, `defaults` will **not** be joined with the options.
///
/// ```toml
/// mount_opts = ["compress=zstd"]
/// ```
///
/// `usage` - Usage of the partition
/// --------------------------------
///
/// Describes the intended use of the partition.
///
/// Possible values are:
///
/// - `boot`: Boot partition. Only one boot partition is allowed, and will be marked as active if MBR is used.
/// - `rootfs`: Root filesystem. Only one root partition is allowed.
/// - `data`: Data partition.
/// - `Other`: Other uses.
///
/// Examples
/// ========
///
/// Raspberry Pi 4 and onwards
/// --------------------------
///
/// Here's an example for the Raspberry Pi 4 and later models.
///
/// Raspberry Pi 4 and later models supports GPT partition table. The OS image contains one boot partition formatted as FAT32, and one partition for the root filesystem formatted as ext4.
///
/// ```toml
/// # The boot partition
/// [[partition]]
/// num = 1
/// type = "esp"
/// size_in_sectors = 614400
/// mountpoint = "/boot/rpi"
/// filesystem = "fat32"
/// usage = "boot"
///
/// # The root partition
/// [[partition]]
/// num = 2
/// type = "linux"
/// size_in_sectors = 0
/// mountpoint = "/"
/// filesystem = "ext4"
/// usage = "rootfs"
/// ```
///
/// Rockchip boards
/// ---------------
///
/// Rockchip SoCs commonly uses the following partitioning scheme:
///
/// ```toml
/// # Partition containing U-Boot TPL and SPL, 7.9MiB
/// [[partition]]
/// num = 1
/// start_sector = 64
/// # Let's assume it is a basic data partition.
/// type = "basic"
/// size_in_sectors = 16320
/// label = "U-Boot-TPL"
/// usage = "other"
///
/// # Partition containing the U-Boot proper, 8MiB
/// [[partition]]
/// num = 2
/// start_sector = 16384
/// type = "basic"
/// size_in_sectors = 16384
/// label = "U-Boot"
/// usage = "other"
///
/// # EFI System Partition used by U-Boot, 300MiB
/// [[partition]]
/// num = 3
/// type = "esp"
/// size_in_sectors = 614400
/// label = "EFI"
/// filesystem = "fat32"
/// mountpoint = "/efi"
/// usage = "boot"
///
/// # Root filesystem, btrfs with ZStandard compression
/// [[partition]]
/// num = 4
/// type = "linux"
/// label = "Root"
/// size_in_sectors = 0
/// filesystem = "btrfs"
/// mount_opts = ["compress=zstd"]
/// mountpoint = "/"
/// usage = "rootfs"
/// ```
///
/// [Device Specification File]: crate::device::DeviceSpec
/// [`"efi"`]: PartitionType::EFI
/// [`"linux"`]: PartitionType::Linux
/// [`"swap"`]: PartitionType::Swap
/// [`"basic"`]: PartitionType::Basic
/// [`"uuid"`]: PartitionType::Uuid
/// [`"byte"`]: PartitionType::Byte

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PartitionSpec {
	#[serde(alias = "no")]
	pub num: u32,
	#[serde(rename = "type", flatten)]
	pub part_type: PartitionType,
	pub start_sector: Option<u64>,
	pub size_in_sectors: u64,
	pub label: Option<String>,
	pub mountpoint: Option<String>,
	pub filesystem: FilesystemType,
	pub mount_opts: Option<Vec<String>>,
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
			Self::Uuid { .. } => Err(anyhow!("Can not convert an arbitrary byte to UUID.")),
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
			Self::Nested { .. } => Err(anyhow!("Nested partition tables are not supported.")),
		}
	}
}

#[cfg(test)]
mod tests {
	const TEST_EFI: &str = r#"type = "efi""#;
	const TEST_LINUX: &str = r#"type = "linux""#;
	const TEST_SWAP: &str = r#"type = "swap""#;
	const TEST_BASIC: &str = r#"type = "basic""#;
	// const TEST_INVALID: &str = r#"type = "whatever""#;
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
