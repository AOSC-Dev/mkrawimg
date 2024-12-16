use core::time;
use std::{
	fs::{create_dir_all, File},
	path::{Path, PathBuf},
	thread,
};

use anyhow::{anyhow, Context, Result};
use clap::ValueEnum;
use gptman::{GPTPartitionEntry, GPT};
use log::{debug, info};
use loopdev::LoopControl;
use mbrman::{MBRPartitionEntry, CHS, MBR};
use strum::{Display, VariantArray};
use sys_mount::{unmount, Mount, UnmountFlags};
use termsize::Size;
use uuid::Uuid;

use crate::{
	cli::Compression,
	device::{DeviceSpec, PartitionMapType},
	filesystem::FilesystemType,
	partition::PartitionUsage,
	utils::{create_sparse_file, refresh_partition_table, restore_term, rsync_sysroot, sync_filesystem},
};

#[derive(Copy, Clone, Debug, Display, PartialEq, Eq, PartialOrd, Ord, ValueEnum, VariantArray)]
pub enum ImageVariant {
	Base,
	Desktop,
	Server,
}

/// A context, or a job that builds an image.
/// Everything is static (the device specs are immutable after being
/// assembled into DeviceRegistry).
#[allow(dead_code)]
pub struct ImageContext<'a> {
	pub device: &'a DeviceSpec,
	pub variant: &'a ImageVariant,
	pub workdir: &'a Path,
	pub outdir: &'a Path,
	// Filename can not be a ref unless there's another thing that
	// holds the (rather unique) filename during execution, since
	// the filename is combined with several pieces.
	pub filename: String,
	pub base_dist: PathBuf,
	pub override_rootfs_fstype: &'a Option<FilesystemType>,
	pub additional_packages: &'a Option<Vec<String>>,
	pub compress: &'a Compression,
}

pub type ImageContextQueue<'a> = Vec<ImageContext<'a>>;

impl ImageContext<'_> {
	fn info<S: AsRef<str>>(&self, content: S) -> () {
		let content = content.as_ref();
		info!(
			"[{} {}] {}",
			&self.device.id,
			&self.variant.to_string().to_lowercase(),
			content
		);
	}

	fn partition_gpt(&self, img: &Path) -> Result<()> {
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
				return Err(anyhow!("Partition number must start from 1."));
			}
			let rand_part_uuid = Uuid::new_v4();
			let unique_partition_guid = rand_part_uuid.to_bytes_le();
			let free_blocks = new_table.find_free_sectors();
			debug!("Free blocks remaining: {:#?}", &free_blocks);
			let last_free = free_blocks
				.last()
				.context("No more free space available for new partitions")?;
			let size =
				if partition.size != 0 {
					partition.size
				} else {
					if partition.num != num_partitions {
						return Err(anyhow!("Max sized partition must stay at the end of the table."));
					}
					if last_free.1 < 1048576 / sector_size {
						return Err(anyhow!("Not enough free space to create a partition"));
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

	fn partition_mbr(&self, img: &Path) -> Result<()> {
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
				return Err(anyhow!("Partition number must start from 1."));
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
					return Err(anyhow!("Max sized partition must stay at the end of the table."));
				}
				last_free.1 - 1
			};
			if sectors < 1048576 / sector_size {
				return Err(anyhow!("Not enough free space to create a partition"));
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

	#[inline]
	pub fn partition_image<P: AsRef<Path>>(&self, dev: P) -> Result<()> {
		let disk_path = dev.as_ref();
		match &self.device.partition_map {
			PartitionMapType::GPT => self.partition_gpt(disk_path),
			PartitionMapType::MBR => self.partition_mbr(disk_path),
			// _ => {
			// 	return Err(anyhow!("Unsupported partition map"));
			// }
		}?;
		self.info(format!("Informing the kernel to reload the partition table on {} ...", disk_path.display()));
		// FIXME: BLKRRPART ioctl call EINVALs on loop devices.
		// For now we call partprobe to tell the kernel to reread the partition table.
		// gptman::linux::reread_partition_table(&mut File::options().read(true).write(true).open(disk_path)?)?;
		refresh_partition_table(dev)?;
		Ok(())
	}

	#[inline]
	pub fn format_partitions(&self, loopdev: &dyn AsRef<Path>) -> Result<()> {
		let loopdev = loopdev.as_ref();
		for partition in &self.device.partitions {
			if partition.filesystem == FilesystemType::Null {
				continue;
			}
			self.info(format!(
				"Formatting partition {} ({:?})",
				partition.num, &partition.filesystem
			));
			let num = partition.num;
			let part_path = format!("{}p{}", loopdev.to_string_lossy(), num);
			let label = &partition.label;
			let mut command = partition
				.filesystem
				.get_mkfs_cmdline(&part_path, label.to_owned())?;
			let status = command.status()?;
			if !status.success() {
				return Err(anyhow!(
					"Command {:?} exited with non-zero status {}",
					command,
					status.code().unwrap_or(1)
				));
			}
		}
		Ok(())
	}

	pub fn mount_partitions<P: AsRef<Path>>(
		&self,
		loop_dev: P,
		mntdir_base: P,
		stack: &mut Vec<String>,
	) -> Result<()> {
		let mntdir_base = mntdir_base
			.as_ref()
			.canonicalize()
			.context("Failed to canonicalize the mount base directory")?;
		let loop_dev = loop_dev.as_ref();
		debug!("Base directory for mountpoints: {}", mntdir_base.display());
		for partition in &self.device.partitions {
			if partition.filesystem == FilesystemType::Null {
				continue;
			}
			let src_dir = format!("{}p{}", loop_dev.to_string_lossy(), partition.num);
			let src_dir = Path::new(&src_dir);
			let dst_dir = mntdir_base.join(format!("p{}", partition.num));
			create_dir_all(&dst_dir)?;
			debug!("Mounting {} to {}", src_dir.display(), dst_dir.as_path().display());
			let mount = Mount::builder().fstype(partition.filesystem.get_os_fstype()?);
			mount.mount(src_dir, &dst_dir)?;
			stack.push(dst_dir.to_string_lossy().to_string());
		}
		Ok(())
	}

	pub fn mount_partitions_in_root<P: AsRef<Path>>(
		&self,
		loop_dev: P,
		rootdir: P,
		stack: &mut Vec<String>,
	) -> Result<()> {
		let loop_dev = loop_dev.as_ref();
		let rootdir = rootdir.as_ref();
		for partition in &self.device.partitions {
			if partition.filesystem == FilesystemType::Null {
				continue;
			}
			if partition.usage == PartitionUsage::Rootfs {
				continue;
			}
			if let Some(mp) = &partition.mountpoint {
				let src_dir =format!("{}p{}", loop_dev.to_string_lossy(), partition.num);
				let src_dir = Path::new(&src_dir);
				// Joining paths with a leading slash replaces the whole path
				let dst_dir = rootdir.join(mp.trim_start_matches('/'));
				create_dir_all(&dst_dir)?;
				let mount = Mount::builder().fstype(partition.filesystem.get_os_fstype()?);
				mount.mount(src_dir, &dst_dir)?;
				stack.push(dst_dir.to_string_lossy().to_string());
			}
		}
		Ok(())
	}

	#[inline]
	pub fn umount_stack(stack: &mut Vec<String>) -> Result<()> {
		loop {
			let cur = stack.pop();
			if let Some(s) = cur {
				debug!("Syncing filesystem {} ...", &s);
				sync_filesystem(&s)?;
				debug!("Umounting {} ...", &s);
				let p = Path::new(&s);
				unmount(p, UnmountFlags::empty())?;
				thread::sleep(time::Duration::from_millis(100));
			} else {
				// exhausted
				break;
			}
		}
		Ok(())
	}

	pub fn postinst_step<P: AsRef<Path>>(&self, postinst_path: P, rootdir: P) -> Result<()> {
		let postinst_path = postinst_path.as_ref();
		let rootdir = rootdir.as_ref();

		Ok(())
	}

	pub fn execute(self, num: usize, len: usize) -> Result<()> {
		let draw_progressbar = |content: &str| {
			// we don't want to screw up the terminal.
			let size = termsize::get().unwrap_or(Size { rows: 25, cols: 80 });
			eprint!("\x1b7\x1b[{};0f\x1b[42m\x1b[0K\x1b[2K", size.rows);
			eprint!(
				"\x1b[30m[{}/{}] {} ({:?}): {}",
				num, len, &self.device.id, &self.variant, content
			);
			eprint!("\x1b8");
		};

		let term_geometry = termsize::get().unwrap_or(Size { rows: 25, cols: 80 });
		// Set up the scroll region
		eprint!("\n\x1b7\x1b[0;{}r\x1b8\x1b[1A", term_geometry.rows - 1);
		let workdir_base = self
			.workdir
			.join(format!("{}-{}", &self.device.id, &self.variant));
		let outdir_base = self.outdir.join(format!(
			"os-{}/{}/rawimg/{}",
			&self.device.arch.to_string().to_lowercase(),
			&self.variant.to_string().to_lowercase(),
			&self.device.vendor
		));
		let mountdir_base = workdir_base.join("mnt");
		let size = self.device.size.get_variant_size(&self.variant) * (1 << 20);
		let mut mountpoint_stack: Vec<String> = Vec::new();
		let mut root_dev_num = None;
		for p in &self.device.partitions {
			if p.usage == PartitionUsage::Rootfs {
				root_dev_num = Some(p.num);
			}
		}
		if root_dev_num.is_none() {
			return Err(anyhow!("Unable to find a root filesystem"));
		}
		let root_dev_num = root_dev_num.unwrap();
		self.info(format!(
			"Image:\n\t\"{}\" ({}) - {}",
			&self.device.name, &self.device.id, &self.variant
		));

		self.info(format!("Output file:\n\t{}", &self.filename));
		self.info("Initializing image ...");
		draw_progressbar("Initializing image");
		// Create workdir_base and all its parents.
		debug!(
			"Creating directory '{}' and all of its parents ...",
			&workdir_base.display()
		);
		create_dir_all(&workdir_base)?;
		// Create outdir_base and all its parents.
		debug!(
			"Creating directory '{}' and all of its parents ...",
			&workdir_base.display()
		);
		create_dir_all(&outdir_base)?;
		create_dir_all(&mountdir_base)?;
		let rawimg_path = (&workdir_base).join("rawmedia.img");
		create_sparse_file(&rawimg_path, size)?;

		// Attach to a loop device
		debug!("Getting fd on /dev/loop-control ...");
		let loop_ctl = LoopControl::open()?;
		debug!("Finding available loop device ...");
		let loop_dev = loop_ctl
			.next_free()
			.context("No available loop device found")?;
		loop_dev.attach_file(&rawimg_path)?;
		let loop_dev_path = loop_dev
			.path()
			.context("Unable to get the path of the loop device")?;
		debug!(
			"Attacthed raw image file {} to {}",
			&rawimg_path.display(),
			&loop_dev_path.display()
		);

		self.info(format!("Creating partitions ..."));
		self.partition_image(&loop_dev_path)
			.context("Failed to partition the image")?;

		// Command::new("lsblk").spawn()?;

		self.info(format!("Formating partitions ..."));
		self.format_partitions(&loop_dev_path)?;
		let rootpart_path = format!("{}p{}", &loop_dev_path.to_string_lossy(), root_dev_num);
		let rootfs_path = mountdir_base.join(format!("p{}", root_dev_num));

		self.info("Mounting partitions ...");
		self.mount_partitions(&loop_dev_path, &mountdir_base, &mut mountpoint_stack)?;

		thread::sleep(time::Duration::from_secs(1));
		self.info(format!("Installing system distribution ..."));
		draw_progressbar("Installing base distribution");
		rsync_sysroot(&self.base_dist, &rootfs_path)?;
		self.mount_partitions_in_root(&loop_dev_path, &rootfs_path, &mut mountpoint_stack)?;

		thread::sleep(time::Duration::from_secs(1));
		self.info(format!("Installing BSP packages ..."));
		draw_progressbar("Installing packages");
		thread::sleep(time::Duration::from_secs(1));
		let device_spec_path = self.device.file_path.to_owned();
		let postinst_script_dir = device_spec_path.parent().context("Unable to find the directory containing the device spec")?;
		let mut postinst_script_path = (&postinst_script_dir).join("postinst.bash");
		if !postinst_script_path.is_file() {
			postinst_script_path = (&postinst_script_dir).join("postinst.sh");
		};
		if !postinst_script_path.is_file() {
			postinst_script_path = (&postinst_script_dir).join("postinst");
		}
		if postinst_script_path.is_file() {
			self.info(format!("Running post installation step ..."));
			draw_progressbar("Post installation step");
			thread::sleep(time::Duration::from_secs(1));
		} else {
			self.info("No postinst script found, skipping.");
		}
		self.info(format!("Finishing up ..."));
		draw_progressbar("Finishing up");
		self.info("Unmounting filesystems ...");
		ImageContext::<'_>::umount_stack(&mut mountpoint_stack)?;
		thread::sleep(time::Duration::from_secs(1));
		self.info("Detaching the loop device ...");
		loop_dev.detach()?;
		// fs::remove_file(rawimg_path)?;
		thread::sleep(time::Duration::from_secs(10));
		restore_term();
		Ok(())
	}
}
