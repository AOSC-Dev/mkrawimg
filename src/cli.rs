//! Module defining the command line usage.
//!
//! Available subcommands
//! ---------------------
//!
//! ### List Available Devices
//!
//! ```shell
//! $ ./target/release/mkrawimg list --format FORMAT
//! ```
//!
//! While `FORMAT` can be one of the following:
//!
//! - `pretty`: table format which contains basic information.
//! - `simple`: simple column-based format splitted by tab character (`'\t'`).
//!
//! ### Build images for one specific device
//!
//! <div class="warning">
//! Building images requires the root privileges.
//! </div>
//!
//! ```shell
//! # ./target/release/mkrawimg build --variants VARIANTS DEVICE
//! ```
//!
//! - `VARIANTS`: distribution variants, can be one or more of the `base`, `desktop`, `server`.
//!   If not specified, all variants will be built.
//! - `DEVICE`: A string identifying the target device, can be one of the following:
//!   - Device ID (defined in `device.toml`).
//!   - Device alias (defined in `device.toml`).
//!   - The path to the `device.toml` file.
//!
//! ### Build Images for All Devices (in the registry)
//!
//! <div class="warning">
//! Building images requires the root privileges.
//! </div>
//!
//! ```shell
//! # ./target/release/mkrawimg build-all --variants VARIANTS
//! ```
//!
//! ### Check validity of the device specification files
//!
//! ```shell
//! $ ./target/release/mkrawimg check
//! ```
//!
//! For the advanced usage, please go to [`Cmdline`].
use std::{path::PathBuf, vec};

use clap::{ArgAction, Parser, Subcommand, ValueEnum};

use crate::context::ImageVariant;

/// Overrides the filesystem type of the root filesystem.
///
/// If not specified, the filesystem type defined in the device specification will be used.
/// ```shell
/// ./target/release/mkrawimg build --fstype xfs
/// ```
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum RootFsType {
	Ext4,
	Btrfs,
	Xfs,
}

/// Specifies the compression format for the output image.
///
/// The default compression format is `xz`.
///
/// Available formats:
///
/// - `xz`: LZMA2 compression (using the xz format). Output filename extension: `.img.xz`
/// - `zstd`: ZStandard compression. Output filename extension: `.img.zst`
/// - `gzip`: DEFLATE compression (using the gzip format). Output filename extension: `.img.gz`
/// - `none`: No compression. Output filename extension: `.img`
#[derive(Copy, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum Compression {
	/// LZMA2 compression (using the xz format). Output filename extension: `.img.xz`
	Xz,
	/// ZStandard compression. Output filename extension: `.img.zst`
	Zstd,
	/// DEFLATE compression (using the gzip format). Output filename extension: `.img.gz`
	Gzip,
	/// No compression. Output filename extension: `.img`
	None,
}

#[derive(Clone, ValueEnum)]
pub enum ListFormat {
	Pretty,
	Simple,
	// Json,
}

/// Command line usage
/// ==================
///
/// This tool uses the subcommand approach to specify the action to take.
///
/// ```shell
/// ./target/release/mkrawimg [GLOBAL_OPTIONS] action [OPTIONS] [--] ARG [ARG..]
/// ```
///
/// Global options
/// ==============
///
/// - `-D`, `--debug`: Enables the debug output.
/// - `-r`, `--registry`: Overrides the path to the [device registry].
/// - `-W`, `--workdir`: Overrides the working directory path. The default path is `./work`.
/// - `-O`, `--outdir`: Overrides the output directory path. The default path is `./out`.
/// - `-M`, `--mirror`: Overrides the package repository mirror for package downloads. The default mirror is the AOSC OS upstream mirror.
/// - `-U`, `--user`: Overrides the username of the built-in user. The default username is `aosc`.
/// - `-P`, `--password`: Overrides the password of the built-in user. The default password is `anthon`.
///
/// Actions
/// =======
///
/// - `build`: Build images for one specific device.
/// - `build-all`: Build images for all devices registered in the registry.
/// - `check`: Check the validity of the device specification files.
/// - `list`: List all of the devices registered in the registry.
///
/// Notes
/// -----
///
/// - All actions has their specific options, please refer the action's documentation for available options.
/// - Some actions may require addition arguments following the options.
///
/// Action `build`
/// ==============
///
/// This action builds images for one specific device.
///
/// ```shell
/// ./target/release/mkrawimg [GLOBAL_OPTIONS] build [OPTIONS] [--] DEVICE
/// ```
///
/// Options for `build`
/// -------------------
///
/// - `-f`, `--fstype` `FSTYPE`
///
///   Override the filesystem type of the root filesystem.
///
///   Possible values are: `btrfs`, `ext4`, `xfs`.
///
/// - `-c`, `--compression` `COMPRESSION`
///
///   Specify the compression format of the output image.
///
///   Possible values are: `xz`, `zstd`, `gzip`, `none`. The default is `xz`.
///
/// - `-V`, `--variants` `VARIANT [VARIANT...]`
///
///   Select distribution variants to build, must specify at least one variant.
///
///   Possible values are: `base`, `desktop`, `server`. If not specified, all variants will be built.
///
/// - `-r`, `--revision` `REVISION`
///
///   Use a positive integer as the revision of the image. The revision will be added to the filename of the output.
///
/// - `-p`, `--additional-packages` `PKG [PKG...]`
///
///   Supply a list of package names to install into the target system. This does not override the defined list.
///
/// Arguments for `build`
/// ---------------------
///
/// The `build` action takes exactly one argument: `DEVICE`.
///
/// `DEVICE` is a string identifying the target device. It can be one of the following:
/// - A device ID defined in the device specification file.
/// - A device alias defined in the device specification file.
/// - Path to the device specification file `device.toml`.
/// - Path to the directory containing the `device.toml`. The specification file must reside directly within this directory.
///
/// Usage examples for `build`
/// --------------------------
///
/// Suppose the registry at `./devices` contains a device with ID `rpi-5b` and a few aliases: `pi5`, `pi5b`. This device represents the Raspberry Pi 5 Model B. The specification file is located at `raspberrypi/rpi-5b/device.toml` within the registry.
///
/// The following usages are all valid:
///
/// ```bash
/// # Use device ID or alias
/// mkrawimg build rpi-5b
/// mkrawimg build pi5
/// mkrawimg build pi5b
/// # Use the path to the specification file
/// mkrawimg build ./devices/raspberrypi/rpi-5b/device.toml
/// # Supply the (relative) path to the directory containing the target device specification file
/// mkrawimg build raspberrypi/rpi-5b
/// ```
///
/// By default images are built for all distribution variants. You can override this by using `-V` or `--variants`:
///
/// ```bash
/// # Build for the base variant only
/// mkrawimg build -V base -- rpi-5b
/// # Build for the base and desktop variants
/// mkrawimg build -V base -V desktop -- rpi-5b
/// # Same way as above
/// mkrawimg build -V base desktop -- rpi-5b
/// ```
///
/// <div class="warning">
///
/// You must use `--` to delimit the variants from the device argument. Otherwise `"rpi-5b"` will be incorrectly parsed as a variant, leading to errors.
///
/// </div>
///
/// Action `build-all`
/// ==================
///
/// This action builds images for all devices within the registry.
///
/// ```shell
/// # ./target/release/mkrawimg [GLOBAL_OPTIONS] build-all [OPTIONS]
/// ```
///
/// The `build-all` action takes the same options as the `build` action. [See above](#options-for-build) for available options.
///
/// The `build-all` action takes no arguments.
///
/// Action `check`
/// ==============
///
/// This action checks for the validity of the device specificatoin files.
///
/// ```shell
/// ./target/release/mkrawimg [GLOBAL_OPTIONS] check
/// ```
///
/// `check` action does not take any options besides the global options, and it does not take any arguments.
///
/// Action `list`
/// =============
///
/// This action lists the available devices within the registry.
///
/// ```shell
/// ./target/releases/mkrawimg [--registry REGISTRY] list [OPTIONS]
/// ```
///
/// Other global options are accepted but almost all options except `--registry` are ignored.
///
/// `list` action takes no arguments.
///
/// Options for `list`
/// ------------------
///
/// - `-f`, `--format`
///
///   Specify the list format.
///
///   Possible values are:
///   - `pretty`: A table-like format which shows the basic information of devices.
///   - `simple`: A much simpler format which contains three colums splitted by tab character (`'\t'`), and one device per line.
///
/// [device registry]: crate::registry::DeviceRegistry
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
	/// List all available devices
	List {
		#[arg(short, long, default_value = "pretty")]
		format: ListFormat,
	},
}

#[doc(hidden)]
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
