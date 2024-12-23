#![allow(dead_code)]
#![allow(clippy::upper_case_acronyms)]

use std::path::Path;

use anyhow::Result;
use serde::Deserialize;

use crate::{context::ImageContext, device::DeviceArch, utils::{run_str_script_with_chroot, setup_scroll_region}};

#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Default, Debug, Deserialize, PartialEq, Eq)]
pub enum Distro {
	#[default]
	AOSC,
	Debian,
	Ubuntu,
	ArchLinux,
	Fedora,
}

pub enum APT {}
pub enum Oma {}

pub trait PackageManager {
	fn install(packages: &[&str], container: &dyn AsRef<Path>) -> Result<()>;
}

impl PackageManager for APT {
	fn install(packages: &[&str], container: &dyn AsRef<Path>) -> Result<()> {
		// Let's do this the easy way.
		// FIXME might have to fork() and exec() ourselves.
		let mut argv = Vec::<&str>::from([
			"apt-get",
			"install",
			"--yes",
			"-o",
			"Dpkg::Options::=--force-confnew",
			"--",
		]);
		argv.extend_from_slice(packages);
		let mut script = String::from("export DEBIAN_FRONTEND=noninteractive;");
		script += &argv.join(" ");
		// chroot $CONTAINER bash -c "export DEBIAN_FRONTEND=noninteractive;apt-get install --yes -o Dpkg::Options::=--force-confnew pkgs ..."
		run_str_script_with_chroot(container, &script, None)
	}
}

impl PackageManager for Oma {
	fn install(packages: &[&str], container: &dyn AsRef<Path>) -> Result<()> {
		let mut argv = Vec::from([
			"oma",
			"--no-check-dbus",
			"install",
			"--no-progress",
			"--no-refresh-topics",
			"--force-confnew",
			"--yes",
			"--",
		]);
		argv.extend_from_slice(packages);
		run_str_script_with_chroot(container, &argv.join(" "), None)
	}
}

#[inline]
fn install_packages_aosc(
	packages: &[&str],
	container: &dyn AsRef<Path>,
	arch: &DeviceArch,
) -> Result<()> {
	if arch.is_native() {
		Oma::install(packages, container)
	} else {
		match arch {
			DeviceArch::Riscv64 | DeviceArch::Mips64r6el => {
				APT::install(packages, container)
			}
			_ => Oma::install(packages, container),
		}
	}
}

impl ImageContext<'_> {
	pub fn install_packages<P: AsRef<Path>>(
		&self,
		packages: &[&str],
		container: P,
	) -> Result<()> {
		if packages.is_empty() {
			return Ok(());
		}
		match &self.device.distro {
			Distro::AOSC => {
				install_packages_aosc(packages, &container, &self.device.arch)?
			}
			Distro::Debian => todo!(),
			Distro::Ubuntu => todo!(),
			Distro::ArchLinux => todo!(),
			Distro::Fedora => todo!(),
		}
		setup_scroll_region();
		Ok(())
	}
}
