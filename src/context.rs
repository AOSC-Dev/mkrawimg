use core::time;
use std::{
	fs::{create_dir_all, File},
	io::{copy, BufReader, BufWriter, Write},
	path::{Path, PathBuf},
	thread,
	time::{Duration, Instant},
};

use crate::{
	cli::Compression,
	device::{DeviceSpec, PartitionMapType},
	filesystem::FilesystemType,
	partition::PartitionUsage,
	utils::{
		add_user, create_sparse_file, refresh_partition_table, restore_term, rsync_sysroot,
		set_locale, sync_filesystem,
	},
};
use anyhow::{bail, Context, Result};
use clap::ValueEnum;
use log::{debug, info, warn};
use loopdev::LoopControl;
use strum::{Display, VariantArray};
use sys_mount::{unmount, Mount, UnmountFlags};
use termsize::Size;

#[derive(Copy, Clone, Debug, Display, PartialEq, Eq, PartialOrd, Ord, ValueEnum, VariantArray)]
pub enum ImageVariant {
	Base,
	Desktop,
	Server,
}

/// A context, or a job that builds an image.
/// Everything is static (the device specs are immutable after being
/// assembled into [`crate::registry::DeviceRegistry`]).
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
	pub(crate) fn info<S: AsRef<str>>(&self, content: S) -> () {
		let content = content.as_ref();
		info!(
			"[{} {}] {}",
			&self.device.id,
			&self.variant.to_string().to_lowercase(),
			content
		);
	}
	pub(crate) fn warn<S: AsRef<str>>(&self, content: S) -> () {
		let content = content.as_ref();
		warn!(
			"[{} {}] {}",
			&self.device.id,
			&self.variant.to_string().to_lowercase(),
			content
		);
	}

	#[inline]
	fn partition_image<P: AsRef<Path>>(&self, dev: P) -> Result<()> {
		let disk_path = dev.as_ref();
		match &self.device.partition_map {
			PartitionMapType::GPT => self.partition_gpt(disk_path),
			PartitionMapType::MBR => self.partition_mbr(disk_path),
			// _ => {
			// 	bail!("Unsupported partition map");
			// }
		}?;
		self.info(format!(
			"Informing the kernel to reload the partition table on {} ...",
			disk_path.display()
		));
		// FIXME: BLKRRPART ioctl call EINVALs on loop devices.
		// For now we call partprobe to tell the kernel to reread the partition table.
		// gptman::linux::reread_partition_table(&mut File::options().read(true).write(true).open(disk_path)?)?;
		refresh_partition_table(dev)?;
		Ok(())
	}

	fn mount_partitions<P: AsRef<Path>>(
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
			debug!(
				"Mounting {} to {}",
				src_dir.display(),
				dst_dir.as_path().display()
			);
			let mount =
				Mount::builder().fstype(partition.filesystem.get_os_fstype()?);
			mount.mount(src_dir, &dst_dir)?;
			stack.push(dst_dir.to_string_lossy().to_string());
		}
		Ok(())
	}

	fn mount_partitions_in_root<P: AsRef<Path>>(
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
				let src_dir =
					format!("{}p{}", loop_dev.to_string_lossy(), partition.num);
				let src_dir = Path::new(&src_dir);
				// Joining paths with a leading slash replaces the whole path
				let dst_dir = rootdir.join(mp.trim_start_matches('/'));
				create_dir_all(&dst_dir)?;
				let mount = Mount::builder()
					.fstype(partition.filesystem.get_os_fstype()?);
				mount.mount(src_dir, &dst_dir)?;
				stack.push(dst_dir.to_string_lossy().to_string());
			}
		}
		Ok(())
	}

	#[inline]
	fn umount_stack(stack: &mut Vec<String>) -> Result<()> {
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

	fn postinst_step<P: AsRef<Path>>(&self, postinst_path: P, rootdir: P) -> Result<()> {
		let postinst_path = postinst_path.as_ref();
		let rootdir = rootdir.as_ref();

		Ok(())
	}

	fn compress_image<P: AsRef<Path>>(&self, from: P, to: P) -> Result<()> {
		let from = from.as_ref();
		let to = to.as_ref();
		let from_fd = File::options().read(true).open(&from)?;
		let to_fd = File::options().write(true).create(true).open(&to)?;

		let num_cpus = num_cpus::get().clamp(1, 32) as u32;

		let start: Instant;
		let duration: Duration;

		match &self.compress {
			Compression::None => {
				self.info(format!("Not compressing the raw image as instructed, copying the raw image to {} ...", &to.display()));
			}
			_ => {
				self.info(format!(
					"Compressing the raw image to {} using {:?} ...",
					&to.display(),
					&self.compress
				));
				if self.compress != &Compression::Gzip {
					self.info(format!(
						"Using {} threads for compression",
						num_cpus
					));
				}
			}
		}
		match &self.compress {
			Compression::Xz => {
				let mut bufreader = BufReader::with_capacity(1048576, from_fd);
				let mut xz_filter = xz2::stream::Filters::new();
				let mut xz_options = xz2::stream::LzmaOptions::new_preset(9)?;
				xz_options.nice_len(273);
				xz_filter.lzma2(&xz_options);
				let encoder = xz2::stream::MtStreamBuilder::new()
					.filters(xz_filter)
					.threads(num_cpus)
					.block_size(1048576)
					.check(xz2::stream::Check::Crc32)
					.encoder()?;
				let mut writer = xz2::write::XzEncoder::new_stream(to_fd, encoder);
				start = Instant::now();
				copy(&mut bufreader, &mut writer)?;
				writer.finish()?.flush()?;
				duration = start.elapsed();
			}
			Compression::Zstd => {
				// zstd::stream::copy_encode(from_fd, to_fd, 9)?;
				let mut bufreader = BufReader::with_capacity(1048576, from_fd);
				let mut writer = zstd::stream::Encoder::new(to_fd, 9)?;
				writer.multithread(num_cpus)?;
				start = Instant::now();
				copy(&mut bufreader, &mut writer)?;
				writer.finish()?.flush()?;
				duration = start.elapsed();
			}
			Compression::Gzip => {
				self.warn("Caution! GZip does not support multi-threading. Compression will be very slow.");
				let bufreader = BufReader::with_capacity(1048576, from_fd);
				let mut encoder = flate2::bufread::GzEncoder::new(
					bufreader,
					flate2::Compression::new(9),
				);
				let mut bufwriter = BufWriter::with_capacity(1048576, to_fd);
				start = Instant::now();
				copy(&mut encoder, &mut bufwriter)?;
				duration = start.elapsed();
			}
			Compression::None => {
				// Using std::fs::copy.
				self.info("No compression specified, copying file directly.");
				// Close the files first.
				drop(from_fd);
				drop(to_fd);
				std::fs::copy(from, to)?;
				self.info("Done copying the raw image.");
				return Ok(());
			}
		}
		self.info(format!(
			"Compression finished in {:.2} seconds.",
			duration.as_secs_f64()
		));
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
		let outfile_path = outdir_base.join(&self.filename);
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
			bail!("Unable to find a root filesystem");
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
		let rootpart_path =
			format!("{}p{}", &loop_dev_path.to_string_lossy(), root_dev_num);
		let rootfs_path = mountdir_base.join(format!("p{}", root_dev_num));

		self.info("Mounting partitions ...");
		self.mount_partitions(&loop_dev_path, &mountdir_base, &mut mountpoint_stack)?;

		self.info(format!("Installing system distribution ..."));
		draw_progressbar("Installing base distribution");
		rsync_sysroot(&self.base_dist, &rootfs_path)?;
		self.mount_partitions_in_root(&loop_dev_path, &rootfs_path, &mut mountpoint_stack)?;

		self.info(format!("Installing BSP packages ..."));
		draw_progressbar("Installing packages");
		let device_spec_path = self.device.file_path.to_owned();
		let postinst_script_dir = device_spec_path
			.parent()
			.context("Unable to find the directory containing the device spec")?;
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
		} else {
			self.info("No postinst script found, skipping.");
		}
		self.info(format!("Finishing up ..."));
		draw_progressbar("Finishing up");
		self.info("Unmounting filesystems ...");
		ImageContext::<'_>::umount_stack(&mut mountpoint_stack)?;
		self.info("Detaching the loop device ...");
		loop_dev.detach()?;
		// fs::remove_file(rawimg_path)?;
		self.compress_image(&rawimg_path, &outfile_path)?;
		restore_term();

		info!("Done! image finished.");
		Ok(())
	}
}
