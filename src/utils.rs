use std::{
	ffi::{c_int, c_void, CString},
	fs::File,
	io::{Seek, Write},
	os::unix::fs::chown,
	path::{Path, PathBuf},
	process::{Command, Stdio},
};

use anyhow::{anyhow, bail, Context, Result};
use blkid::prober::ProbeState;
use libc::{close, open, O_NONBLOCK, O_RDONLY};
use log::{debug, info};
use termsize::Size;
use walkdir::WalkDir;

use crate::{context::ImageVariant, device::DeviceArch};

#[link(name = "c")]
extern "C" {
	#[allow(dead_code)]
	pub fn geteuid() -> c_int;
	#[allow(dead_code)]
	pub fn sync() -> c_void;
	pub fn syncfs(fd: c_int) -> c_int;
}

const AB_DIR: &str = "/usr/share/aoscbootstrap";
const DEFAULT_GROUPS: &[&str] = &["audio", "video", "cdrom", "plugdev", "tty", "wheel"];
const LOCALCONF_PATH: &str = "etc/locale.conf";
const BINFMT_DIR: &str = "/proc/sys/fs/binfmt_misc";

/// Create a sparse file with specified size in bytes.
pub fn get_sparse_file<P: AsRef<Path>>(path: P, size: u64) -> Result<File> {
	let img_path = path.as_ref();
	let parent = img_path.parent().unwrap_or(Path::new("/"));
	if !parent.exists() {
		return Err(anyhow!(
			"One or more of the parent directories does not exist"
		));
	}
	debug!(
		"Creating sparse file at '{}' with size {} bytes ...",
		&img_path.display(),
		size
	);
	let mut img_file = File::create_new(img_path).context(format!(
		"Error creating raw image file '{}'",
		&img_path.display()
	))?;
	// Seek to the desired size
	img_file.seek(std::io::SeekFrom::Start(size - 1))?;
	// Write zero at the end of file to punch a hole
	img_file.write_all(&[0]).context(
		"Failed to punch hole for sparse file. Does your filesystem support sparse files?",
	)?;
	img_file.sync_all()?;
	Ok(img_file)
}

pub fn create_sparse_file<P: AsRef<Path>>(path: P, size: u64) -> Result<()> {
	get_sparse_file(path, size)?;
	Ok(())
}

/// Tell kernel to reread the partition table.
pub fn refresh_partition_table<P: AsRef<Path>>(dev: P) -> Result<()> {
	debug!("Refreshing partition table ...");
	let dev = dev.as_ref();
	let mut command = Command::new("partprobe");
	let command = command.arg("--summary").arg(dev).stdout(Stdio::piped());
	let out = command
		.output()
		.context("Failed to run partprobe(8) to refresh the partition table")?
		.stdout;
	info!("partprobe: {}", String::from_utf8_lossy(&out).trim());
	Ok(())
}

/// Run aoscbootstrap to generate a system release
pub fn bootstrap_distribution<P: AsRef<Path>, S: AsRef<str>>(
	variant: &ImageVariant,
	path: P,
	arch: DeviceArch,
	mirror: S,
) -> Result<()> {
	let path = path.as_ref();
	let mirror = mirror.as_ref();

	// Display a progressbar
	setup_scroll_region();

	let size = termsize::get().unwrap_or(Size { rows: 25, cols: 80 });
	eprint!("\x1b7\x1b[{};0f\x1b[42m\x1b[0K\x1b[2K", size.rows);
	eprint!(
		"\x1b[30m[{}] Bootstrapping release ...",
		variant.to_string().to_lowercase()
	);
	eprint!("\x1b8\x1b[0m");

	info!(
		"Bootstrapping {} system distribution to {} ...",
		variant,
		path.display()
	);
	let mut command = Command::new("aoscbootstrap");
	let command = command
		.arg("stable")
		.arg(path)
		.arg(mirror)
		.arg("-x")
		.args([
			"--config",
			&format!("{}/{}", AB_DIR, "config/aosc-mainline.toml"),
		])
		.args(["--arch", &arch.to_string().to_lowercase()])
		.args(["-s", &format!("{}/{}", AB_DIR, "scripts/reset-repo.sh")])
		.args(["-s", &format!("{}/{}", AB_DIR, "scripts/enable-dkms.sh")])
		.args([
			"--include-files",
			&format!(
				"{}/recipes/mainline/{}-common.lst",
				AB_DIR,
				match &variant {
					ImageVariant::Desktop => "kde".to_owned(),
					_ => variant.to_string().to_lowercase(),
				}
			),
		]);

	debug!("Runnig command {:?} ...", command);
	let status = command.status().context("Failed to run aoscbootstrap")?;
	// Recover the terminal
	restore_term();
	if status.success() {
		info!(
			"Successfully bootstrapped {} distribution.",
			variant.to_string()
		);
		Ok(())
	} else if let Some(c) = status.code() {
		Err(anyhow!("aoscbootstrap exited unsuccessfully (code {})", c))
	} else {
		Err(anyhow!("rsync exited abnormally"))
	}
}

pub fn rsync_sysroot<P: AsRef<Path>>(src: P, dst: P) -> Result<()> {
	let src = src.as_ref();
	let dst = dst.as_ref();
	if !src.is_dir() || !dst.is_dir() {
		bail!("Neither directory exists.");
	}
	info!(
		"Installing the distribution in {} to {} ...",
		src.display(),
		dst.display()
	);
	let mut command = Command::new("rsync");
	command.args(["-axAHXSW", "--numeric-ids", "--info=progress2", "--no-i-r"]);
	command.arg(format!("{}/", src.to_string_lossy()));
	command.arg(format!("{}/", dst.to_string_lossy()));
	debug!("Running command {:?}", command);
	// return Ok(());
	cmd_run_check_status(&mut command)
}

/// Set up the scroll region (for a progress bar on the bottom)
#[inline]
pub fn setup_scroll_region() {
	let term_geometry = termsize::get().unwrap_or(Size { rows: 25, cols: 80 });
	// Set up the scroll region
	eprint!("\n\x1b7\x1b[0;{}r\x1b8\x1b[1A", term_geometry.rows - 1);
}

/// Recover the terminal
#[inline]
pub fn restore_term() {
	let term_geometry = termsize::get().unwrap_or(Size { rows: 25, cols: 80 });
	eprint!(
		"\x1b7\x1b[0;{}r\x1b[{};0f\x1b[0K\x1b8",
		term_geometry.rows, term_geometry.rows
	);
}

#[allow(dead_code)]
pub fn sync_all() -> Result<()> {
	let _ = unsafe { sync() };
	Ok(())
}

/// Sync the filesystem behind the path.
pub fn sync_filesystem(path: &dyn AsRef<Path>) -> Result<()> {
	let tgt_path = path.as_ref();
	let path = CString::new(tgt_path.as_os_str().as_encoded_bytes())?;
	let path_ptr = path.as_ptr();

	let fd = unsafe { open(path_ptr, O_RDONLY | O_NONBLOCK) };
	if fd < 0 {
		let errno = errno::errno();
		return Err(anyhow!(
			"Failed to open path {}: {}",
			&tgt_path.display(),
			errno
		));
	}
	debug!("open(\"{}\") returned fd {}", &tgt_path.display(), fd);
	let result = unsafe { syncfs(fd) };
	debug!("syncfs({}) returned {}", fd, result);
	if result != 0 {
		let close = unsafe { close(fd) };
		if close != 0 {
			panic!("Failed to close fd {}: {}", fd, errno::errno());
		}
		let errno = errno::errno();
		return Err(anyhow!(
			"Failed to sync filesystem {}: {}",
			tgt_path.display(),
			errno
		));
	}
	let close = unsafe { close(fd) };
	if close != 0 {
		panic!("Failed to close fd {}: {}", fd, errno::errno());
	}
	Ok(())
}

pub fn add_user<S, T, P>(
	root: P,
	name: S,
	password: S,
	comment: Option<T>,
	homedir: Option<P>,
	groups: Option<&[&str]>,
) -> Result<()>
where
	S: AsRef<str>,
	T: AsRef<str>,
	P: AsRef<Path>,
{
	// shadow does not expose such functionality through a library,
	// we have to invoke commands to achieve this.
	let name = name.as_ref();
	let root = root.as_ref().to_string_lossy().to_string();
	let password = password.as_ref();
	let comment = comment.as_ref();
	let homedir = if let Some(h) = homedir {
		PathBuf::from(h.as_ref())
	} else {
		PathBuf::from("/home").join(name)
	};
	let homedir = homedir.to_string_lossy();
	let groups = if let Some(g) = groups {
		g
	} else {
		DEFAULT_GROUPS
	};
	let groups = groups.join(",");
	let mut cmd_useradd = Command::new("chroot");
	let mut cmd_chpasswd = Command::new("chroot");
	cmd_useradd
		.arg(&root)
		.arg("useradd")
		.args(["-m", "-d", &homedir])
		.args(["-G", &groups]);
	if let Some(c) = comment {
		cmd_useradd.args(["-c", c.as_ref()]);
	}
	cmd_useradd.arg(name);
	cmd_chpasswd.stdin(Stdio::piped()).args([&root, "chpasswd"]);
	cmd_run_check_status(&mut cmd_useradd)?;
	let mut chpasswd_proc = cmd_chpasswd.spawn().context("Failed to run chpasswd")?;
	let chpasswd_stdin = chpasswd_proc
		.stdin
		.as_mut()
		.context("Failed to open stdin for chpasswd")?;
	// echo "$name:$password" | chpasswd -R /target/root
	let chpasswd_buf = format!("{}:{}", name, password);
	chpasswd_stdin.write_all(chpasswd_buf.as_bytes())?;
	chpasswd_proc.wait()?;
	Ok(())
}

pub fn set_locale<S: AsRef<str>, P: AsRef<Path>>(root: P, locale: S) -> Result<()> {
	let root = root.as_ref();
	let locale = locale.as_ref();
	let locale_conf_path = root.join(LOCALCONF_PATH);
	let locale = format!("LANG=\"{}\"", locale);
	let mut locale_conf_fd = File::options()
		.write(true)
		.truncate(true)
		.create(true)
		.open(locale_conf_path)?;
	locale_conf_fd.write_all(locale.as_bytes())?;
	locale_conf_fd.sync_all()?;
	Ok(())
}

pub fn check_binfmt(arch: &DeviceArch) -> Result<()> {
	if arch.is_native() {
		return Ok(());
	}
	let name = arch.get_qemu_binfmt_names();
	let binfmt_path = Path::new(BINFMT_DIR);
	if !binfmt_path.is_dir() {
		bail!("binfmt_misc support is currently not available on your system. Cannot continue.")
	}
	let path = binfmt_path.join(name);
	if !path.is_file() {
		bail!("{} is not found in registered binfmt_misc targets.\nPlease make sure QEMU static and binfmt file for this target are installed.", name);
	}
	Ok(())
}

pub fn cmd_run_check_status(cmd: &mut Command) -> Result<()> {
	let result = cmd
		.status()
		.context(format!("Failed to run {:?}", cmd.get_program()))?;
	if result.success() {
		Ok(())
	} else if let Some(c) = result.code() {
		Err(anyhow!(
			"The following command failed with exit code {}:\n{:?}",
			c,
			cmd
		))
	} else {
		Err(anyhow!(
			"The following command exited abnormally:\n{:?}",
			cmd
		))
	}
}

pub fn run_str_script_with_chroot(
	root: &dyn AsRef<Path>,
	script: &str,
	shell: Option<&dyn AsRef<str>>,
) -> Result<()> {
	let mut cmd = Command::new("systemd-nspawn");
	let shell = if let Some(s) = shell {
		s.as_ref()
	} else {
		"/bin/bash"
	};
	// Let's assume all shells supports "-c SCRIPT".
	// But I think it is better to pipe into the shell's stdin.
	// bash -c -- script $0 $1 ...
	// The positional param after "-c script" is $0 of that script.
	let script = format!("source /tmp/spec.sh ;{}", script);
	cmd.args([
		"-q",
		"-D",
		&root.as_ref().to_string_lossy(),
		"--",
		shell,
		"-c",
		"--",
		&script,
		"<tmp_script>",
	]);
	cmd_run_check_status(&mut cmd)
}

pub fn run_script_with_chroot<P: AsRef<Path>>(
	root: P,
	script: P,
	shell: Option<&dyn AsRef<str>>,
) -> Result<()> {
	let mut cmd = Command::new("systemd-nspawn");
	let shell = if let Some(s) = shell {
		s.as_ref()
	} else {
		"/bin/bash"
	};
	// Let's assume all shells supports "-c SCRIPT".
	// But I think it is better to pipe into the shell's stdin.
	// bash -c -- script $0 $1 ...
	// The positional param after "-c script" is $0 of that script.
	// We are using 'source' to let the script being run to use the information we provided.
	let full_script = format!(
		"source /tmp/spec.sh ; source {}",
		&script.as_ref().to_string_lossy()
	);
	cmd.args([
		"-q",
		"-D",
		&root.as_ref().to_string_lossy(),
		"--",
		shell,
		"-c",
		"--",
		&full_script,
		// Set $0 to the path of the script
		&script.as_ref().to_string_lossy(),
	]);
	cmd_run_check_status(&mut cmd).context("Failed to run script with chroot")
}

/// Get filesystem UUID of the given block device.
pub fn get_fsuuid(fspath: &dyn AsRef<Path>) -> Result<String> {
	let fspath = fspath.as_ref();
	// WARNING! ACHTUNG!
	// libblkid's cache does not cache loop devices.
	// You will get an EINVAL if you try to use the cache to get FSUUID
	// for filesystems on loop devices, so the following code will not
	// work.
	// let cache = blkid::cache::Cache::new()?;
	// let dev = cache.get_dev(&fspath_str, GetDevFlags::FIND)?;
	// let tags = dev.tags();
	// let uuid_filtered: Vec<_> = tags
	// 	.filter(|x| match x.typ() {
	// 		TagType::Superblock(st) => {
	// 			if st == SuperblockTag::Uuid {
	// 				return true;
	// 			}
	// 			return false;
	// 		}
	// 		_ => {
	// 			return false;
	// 		}
	// 	})
	// 	.collect();

	// We have to do the low-level probing.
	// Wow, somehow the code is simpler.
	let probe = blkid::prober::Prober::new_from_filename(fspath)?;
	let result = probe.do_safe_probe()?;
	match result {
		ProbeState::Success => {
			let x = probe.get_values_map()?;
			let uuid = x.get("UUID").context("No filesystem UUID found in the probe results; Perhaps there's no filesystem in this partition, or the type of the filesystem can't be identified")?;
			Ok(uuid.to_owned())
		}
		_ => {
			bail!("Can not get necessary information of {}", &fspath.display());
		}
	}
}

/// Change the ownership of a filesystem object, recursively.
pub fn return_ownership_recursive(
	path: &dyn AsRef<Path>,
	to_user: Option<u32>,
	to_group: Option<u32>,
) -> Result<()> {
	let walker = WalkDir::new(path.as_ref());
	for entry in walker {
		let entry = entry.context("Loop or unexpected object detected")?;
		chown(entry.path(), to_user, to_group).context(format!(
			"Failed to change the ownership of '{}' to {:?}:{:?}",
			entry.path().display(),
			to_user,
			to_group
		))?;
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::get_fsuuid;
	use anyhow::Result;

	#[test]
	fn test_get_uuid() -> Result<()> {
		let uuid = get_fsuuid(&"/dev/nvme0n1p2")?;
		eprintln!("FSUUID for nvme0n1p2: {}", uuid);
		Ok(())
	}
}
