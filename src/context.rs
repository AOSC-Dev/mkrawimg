use core::time;
use std::{
	fs::{File, create_dir_all},
	io::{BufReader, BufWriter, Write, copy},
	path::{Path, PathBuf},
	thread,
	time::{Duration, Instant},
};

use crate::{
	cli::Compression,
	device::{DeviceArch, DeviceSpec, PartitionMapData, PartitionMapType},
	filesystem::FilesystemType,
	partition::PartitionUsage,
	pm::{APT, Oma, PackageManager},
	topics::{Topic, save_topics},
	utils::{
		add_user, create_sparse_file, fs_zerofill_freespace, refresh_partition_table, restore_term,
		rsync_sysroot, run_script_with_chroot, run_str_script_with_chroot, set_locale,
		setup_scroll_region, sync_filesystem,
	},
};
use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use log::{debug, info, warn};
use loopdev::LoopControl;
use strum::{Display, VariantArray};
use sys_mount::{Mount, UnmountFlags, unmount};
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
	pub user: &'a str,
	pub password: &'a str,
	// Filename can not be a ref unless there's another thing that
	// holds the (rather unique) filename during execution, since
	// the filename is combined with several pieces.
	pub filename: String,
	pub base_dist: PathBuf,
	pub override_rootfs_fstype: &'a Option<FilesystemType>,
	pub additional_packages: &'a Option<Vec<String>>,
	pub compress: &'a Compression,
	pub topics: Option<&'a Vec<Topic>>,
}

pub type ImageContextQueue<'a> = Vec<ImageContext<'a>>;

impl ImageContext<'_> {
	pub(crate) fn info<S: AsRef<str>>(&self, content: S) {
		let content = content.as_ref();
		info!(
			"[{} {}] {}",
			&self.device.id,
			&self.variant.to_string().to_lowercase(),
			content
		);
	}
	pub(crate) fn warn<S: AsRef<str>>(&self, content: S) {
		let content = content.as_ref();
		warn!(
			"[{} {}] {}",
			&self.device.id,
			&self.variant.to_string().to_lowercase(),
			content
		);
	}

	#[inline]
	fn partition_image<P: AsRef<Path>>(&self, dev: P) -> Result<PartitionMapData> {
		let disk_path = dev.as_ref();
		let pm_data = match &self.device.partition_map {
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
		Ok(pm_data)
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
			if partition.filesystem == FilesystemType::None {
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
			// Shoud we handle standard options like ro, nosuid, noexec, etc?
			if let Some(opts) = partition.mount_opts.as_ref() {
				// "defaults" in mount options are ignored.
				let opts: Vec<_> = opts
					.iter()
					.map(|x| x.as_str())
					.filter(|x| x != &"defaults")
					.collect();
				let opts = opts.join(",");
				let mount = Mount::builder()
					.fstype(partition.filesystem.get_os_fstype()?)
					.data(&opts);
				mount.mount(src_dir, &dst_dir)?;
			} else {
				let mount = Mount::builder().fstype(partition.filesystem.get_os_fstype()?);
				mount.mount(src_dir, &dst_dir)?;
			};
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
			if partition.filesystem == FilesystemType::None {
				continue;
			}
			if partition.usage == PartitionUsage::Rootfs {
				continue;
			}
			if let Some(mp) = &partition.mountpoint {
				let src_dir = format!("{}p{}", loop_dev.to_string_lossy(), partition.num);
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

	fn setup_chroot_mounts<P: AsRef<Path>>(
		&self,
		rootdir: P,
		stack: &mut Vec<String>,
	) -> Result<()> {
		let rootdir = rootdir.as_ref();
		let dst = rootdir.join("tmp");
		debug!("Mounting tmpfs to {} ...", &dst.display());
		let mount = Mount::builder().fstype("tmpfs");
		mount.mount("tmpfs", &dst)?;
		stack.push(dst.to_string_lossy().to_string());
		Ok(())
	}

	fn postinst_step<P: AsRef<Path>>(&self, rootdir: P, binds: &[&str]) -> Result<()> {
		let rootdir = rootdir.as_ref();
		// The OOBE Wizard will take care of the user, locale and swapfile setup procedures.
		if !self.device.oobe_wizard {
			self.info("Setting up the user and locale ...");
			add_user(
				rootdir,
				&self.user,
				&self.password,
				Some("Default User"),
				None,
				None,
			)?;
			set_locale(rootdir, "en_US.UTF-8")?;
		} else {
			// Must keep the sanity of the environment.
			set_locale(rootdir, "C.UTF-8")?;
		}
		self.set_hostname(&rootdir)?;

		let postinst_script_dir = self
			.device
			.file_path
			.parent()
			.context("Unable to find the directory containing the device spec")?;
		let mut postinst_script_path = postinst_script_dir.join("postinst.bash");
		if !postinst_script_path.is_file() {
			postinst_script_path = postinst_script_dir.join("postinst.sh");
		};
		if !postinst_script_path.is_file() {
			postinst_script_path = postinst_script_dir.join("postinst");
		}
		if postinst_script_path.is_file() {
			self.info("Running post installation script ...");
			debug!(
				"Copying {} to {} ...",
				&postinst_script_path.display(),
				&rootdir.display()
			);
			let filename = postinst_script_path
				.file_name()
				.context("Unable to get the basename of the script")?;
			let dst_path = &rootdir.join("tmp").join(filename);
			std::fs::copy(&postinst_script_path, dst_path)
				.context("Failed to copy the post installation script")?;
			run_script_with_chroot(rootdir, &Path::new("/tmp").join(filename), binds, None)?;
		} else {
			self.info("No postinst script found, skipping.");
		}

		Ok(())
	}

	fn compress_image<P: AsRef<Path>>(&self, from: P, to: P) -> Result<()> {
		let from = from.as_ref();
		let to = to.as_ref();
		let from_fd = File::options().read(true).open(from)?;
		let to_fd = File::options()
			.write(true)
			.create(true)
			.truncate(true)
			.open(to)?;

		let num_cpus = num_cpus::get().clamp(1, 32) as u32;

		let start: Instant;
		let duration: Duration;

		match &self.compress {
			Compression::None => {
				self.info(format!(
					"Not compressing the raw image as instructed, copying the raw image to {} ...",
					&to.display()
				));
			}
			_ => {
				self.info(format!(
					"Compressing the raw image to {} using {:?} ...",
					&to.display(),
					&self.compress
				));
				if self.compress != &Compression::Gzip {
					self.info(format!("Using {} threads for compression", num_cpus));
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
				self.warn(
					"Caution! GZip does not support multi-threading. Compression will be very slow.",
				);
				let bufreader = BufReader::with_capacity(1048576, from_fd);
				let mut encoder =
					flate2::bufread::GzEncoder::new(bufreader, flate2::Compression::new(9));
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

	fn save_topics(&self, rootdir: &dyn AsRef<Path>) -> Result<()> {
		if let Some(topics) = &self.topics {
			self.info("Saving topics ...");
			save_topics(rootdir.as_ref(), topics)?;
			if !self.device.arch.is_native() && self.device.arch == DeviceArch::mips64r6el {
				APT::upgrade_system(rootdir)?;
			} else {
				Oma::upgrade_system(rootdir)?;
			}
		}
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

		// Set up the scroll region for progressbar.
		setup_scroll_region();

		// Various paths being used
		// The path which used specifically for this task
		// Contains the raw image and the mount points
		let workdir_base = self
			.workdir
			.join(format!("sketches/{}-{}", &self.device.id, &self.variant));
		// The path containing the output
		// Follows the directory hierarchy of AOSC OS releases
		let outdir_base = self.outdir.join(format!(
			"os-{}/{}/rawimg/{}",
			&self.device.arch.to_string().to_lowercase(),
			&self.variant.to_string().to_lowercase(),
			&self.device.vendor
		));
		// The full path to the output file
		let outfile_path = outdir_base.join(&self.filename);
		// Base directory for temporary mount points
		let mountdir_base = workdir_base.join("mnt");
		// Total image size
		let size = self.device.size.get_variant_size(self.variant) * (1 << 20);
		// A stack which remembers all of the active mountpoints
		// These mountpoints must be umounted before this function ends!
		let mut mountpoint_stack: Vec<String> = Vec::new();
		// The index of the partition which contains the root filesystem, in the partition table.
		let mut root_dev_num = None;
		for p in &self.device.partitions {
			if p.usage == PartitionUsage::Rootfs {
				root_dev_num = Some(p.num);
			}
		}
		// If you don't have one, then where the hell do you store the OS?
		if root_dev_num.is_none() {
			bail!("Unable to find a root filesystem");
		}
		let root_dev_num = root_dev_num.unwrap();

		// Begin to produce the image
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
			&outdir_base.display()
		);
		create_dir_all(&outdir_base)?;
		create_dir_all(&mountdir_base)?;
		let rawimg_path = workdir_base.join("rawmedia.img");
		if rawimg_path.is_file() {
			self.warn("Raw image file already exists in the workbench - removing it first.");
			std::fs::remove_file(&rawimg_path)?;
		}
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

		self.info("Creating partitions ...");
		let mut pm_data = self
			.partition_image(&loop_dev_path)
			.context("Failed to partition the image")?;

		self.info("Formating partitions ...");
		self.format_partitions(&loop_dev_path, &mut pm_data)?;

		// Bind mounts to be passed to systemd-nspawn(1).
		// Switching to systemd-nspawn completely eliminates /dev,
		// /sys and /proc bind mounts, but we have to bind mount the
		// loop device the target image is attached to, and all of
		// its partitions to the target, for post installtion and
		// bootloader scripts to access them.
		// We can not bind them beforehand, the only option is to
		// pass `--bind bind1 --bind bind2 ...` to the nspawn
		// command line.
		let mut binds = Vec::new();
		binds.push(loop_dev_path.to_string_lossy().to_string());
		for partition in &self.device.partitions {
			binds.push(format!(
				"{}p{}",
				loop_dev_path.to_string_lossy(),
				partition.num
			));
		}
		let binds = binds.iter().map(|x| x.as_str()).collect::<Vec<_>>();
		let binds = binds.as_slice();

		// The path to the block device which contains the root filesystem.
		let rootpart_dev = format!("{}p{}", &loop_dev_path.to_string_lossy(), root_dev_num);
		self.info("Mounting partitions ...");
		self.mount_partitions(&loop_dev_path, &mountdir_base, &mut mountpoint_stack)?;
		let rootfs_mount = mountdir_base
			.join(format!("p{}", root_dev_num))
			.canonicalize()
			.context("Failed to canonicalize the path of root filesystem mountpoint")?;
		debug!("Root filesystem mountpoint: {:?}", rootfs_mount);

		self.info("Installing system distribution ...");
		draw_progressbar("Installing base distribution");
		rsync_sysroot(&self.base_dist, &rootfs_mount)?;
		self.mount_partitions_in_root(&loop_dev_path, &rootfs_mount, &mut mountpoint_stack)?;
		self.info("Generating fstab ...");
		self.generate_fstab(&pm_data, &rootfs_mount)?;

		self.info("Setting up bind mounts ...");
		self.setup_chroot_mounts(&rootfs_mount, &mut mountpoint_stack)?;

		self.write_spec_script(&loop_dev_path, &rootpart_dev, &rootfs_mount, &pm_data)?;

		self.save_topics(&rootfs_mount)?;

		self.info("Installing BSP packages ...");
		draw_progressbar("Installing packages");
		// Eh we have to "convert" Vec<String> to Vec<&str>.
		let pkgs = &mut self
			.device
			.bsp_packages
			.iter()
			.map(String::as_str)
			.collect::<Vec<&str>>();
		self.install_packages(pkgs.as_slice(), &rootfs_mount)?;

		if let Some(tgt) = &self.device.devena_firstboot_target {
			self.info("Installing devena-firstboot packages ...");
			self.install_packages(&[&format!("devena-firstboot-{}", tgt)], &rootfs_mount)?;
		}

		if self.device.oobe_wizard {
			self.info("Installing OOBE Wizard ...");
			let oobe_package = match self.variant {
				ImageVariant::Desktop => "aosc-os-oobe-gui",
				_ => "aosc-os-oobe-cli"
			};
			self.install_packages(&[&oobe_package], &rootfs_mount)?;
		}

		self.info("Running post installation step ...");
		draw_progressbar("Post installation step");
		self.postinst_step(&rootfs_mount, binds)?;

		if let Some(tgt) = &self.device.devena_firstboot_target {
			self.info("Creating devena-firstboot initramfs images ...");
			run_str_script_with_chroot(
				&rootfs_mount,
				"create-devena-initrd",
				&[],
				Some(&"/bin/bash"),
			)?;
			run_str_script_with_chroot(
				&rootfs_mount,
				&format!(
					"oma remove --no-check-dbus --purge --yes devena-firstboot-{}",
					tgt
				),
				&[],
				Some(&"/bin/bash"),
			)?;
		}

		self.apply_bootloaders(&rootfs_mount, &loop_dev_path, binds)?;

		self.info("Finishing up ...");
		draw_progressbar("Finishing up");
		self.info("Filling the filesystem with zeroes ...");
		fs_zerofill_freespace(&rootfs_mount)?;
		self.info("Unmounting filesystems ...");
		ImageContext::<'_>::umount_stack(&mut mountpoint_stack)?;
		self.info("Detaching the loop device ...");
		loop_dev.detach()?;

		self.compress_image(&rawimg_path, &outfile_path)?;
		restore_term();
		sync_filesystem(&rawimg_path)?;
		info!("Done! image finished.");
		Ok(())
	}
}
