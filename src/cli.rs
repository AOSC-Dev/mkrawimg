use std::{path::PathBuf, vec};

use clap::{ArgAction, Parser, Subcommand, ValueEnum};

use crate::context::ImageVariant;

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum RootFsType {
	Ext4,
	Btrfs,
	Xfs,
}

#[derive(Copy, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum Compression {
	Xz,
	Zstd,
	Gzip,
	None,
}

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cmdline {
	/// Turns on debug output.
	#[arg(long, action = ArgAction::SetTrue)]
	pub debug: bool,
	/// Override path to the device registry
	#[arg(short = 'r', long)]
	pub registry: Option<PathBuf>,
	/// Working directory
	#[arg(short = 'D', long, default_value = "./work")]
	pub workdir: PathBuf,
	/// Output directory
	#[arg(short = 'O', long, default_value = "./out")]
	pub outdir: PathBuf,
	/// The mirror to download packages from.
	#[arg(short = 'm', long, default_value = "https://repo.aosc.io/debs")]
	pub mirror: String,
	/// Specify username for the OS
	#[arg(short = 'U', long, default_value = "aosc")]
	pub user: String,
	/// Specify password for the OS
	#[arg(short = 'P', long, default_value = "anthon")]
	pub password: String,
	/// The action to take.
	#[command(subcommand)]
	pub action: Action,
}

#[derive(Subcommand)]
pub enum Action {
	/// Build images for a device.
	Build {
		/// Override the filesystem type of the root filesystem.
		#[arg(short, long)]
		fstype: Option<RootFsType>,

		/// Image compression format
		#[arg(short, long, value_enum, default_value_t = Compression::Xz)]
		compression: Compression,

		/// Variants to generate (All if not specified)
		#[arg(short = 'V', long, value_enum, num_args = 1.., default_values = vec!["base", "desktop", "server"])]
		variants: Vec<ImageVariant>,

		/// Revision of the image
		#[arg(short, long)]
		revision: Option<u32>,

		/// Additional packages to be installed
		#[arg(short = 'p', long = "packages", num_args = 1..)]
		additional_packages: Option<Vec<String>>,

		/// ID or alias of the target device.
		///
		/// Can be one of the following:
		///
		/// - The exact ID of the device, defined in `device.toml`.
		/// - One of the aliases for the device, defined in `device.toml`.
		/// - Path to the directory containing a `device.toml`.
		/// - Path to the `device.toml` itself.
		#[arg(verbatim_doc_comment)]
		device: String,
	},
	/// Build images for all devices.
	BuildAll {
		/// Override the filesystem type of the root filesystem.
		#[arg(short, long)]
		fstype: Option<RootFsType>,

		/// Image compression format
		#[arg(short, long, value_enum, default_value_t = Compression::Xz)]
		compression: Compression,

		/// Variants to generate (All if not specified)
		#[arg(short = 'V', long, value_enum, num_args = 1.., default_values = vec!["base", "desktop", "server"])]
		variants: Vec<ImageVariant>,

		/// Revision of the image
		#[arg(short, long)]
		revision: Option<u32>,

		/// Additional packages
		#[arg(short = 'p', long = "packages", num_args = 1..)]
		additional_packages: Option<Vec<String>>,
	},
	/// Check for validity of the devices registry.
	Check {
		/// ID or alias of the target device.
		///
		/// Can be one of the following:
		///
		/// - The exact ID of the device, defined in `device.toml`.
		/// - One of the aliases for the device, defined in `device.toml`.
		/// - Path to the directory containing a `device.toml`.
		/// - Path to the `device.toml` itself.
		#[arg(verbatim_doc_comment)]
		device: Option<String>,
	},
}

impl Compression {
	pub fn get_extension(&self) -> &'static str {
		match self {
			Compression::Xz => ".xz",
			Compression::Zstd => ".zst",
			Compression::Gzip => ".gz",
			Compression::None => "",
		}
	}
}
