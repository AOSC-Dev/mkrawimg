//! Module handling the registry of the device specifications.
//!
//! See [`DeviceRegistry`] for details.
use crate::{cli::ListFormat, device::DeviceSpec};
use anyhow::{anyhow, bail, Context, Result};
use log::{debug, error, info};
use owo_colors::OwoColorize;
use std::{
	collections::HashMap,
	fs,
	path::{Path, PathBuf},
};
use walkdir::WalkDir;

/// Device Registry
/// ===============
///
/// A device registry is a directory tree that contains one or more [device specification file]s, and optionally device-related scripts. The registry is read by this tool to find the desired device to build images.
///
/// The registry is organised by vendor and device ID, in a tree-like structure:
///
/// ```plain
/// devices/ # the top-level directory
///   vendor1/ # the vendor-level directory, e.g. "raspberrypi"
///      device1/ # the device-level directory, assume the device ID is 'device1'
///        apply-bootloader.sh    # One of the bootloader scripts for 'device1'
///        apply-bootloader2.sh   # Another bootloader script for 'device1'
///        device.toml            # The device specification file for 'device1'
///        postinst.bash          # The post installation script for 'device1'
///     device2/                  # devices can have no bootloader scripts and/or no post installation scripts
///       apply-bootloader.sh     # The bootloader script for 'device2'
///       device.toml             # The device specification file for 'device2'
///     device3/                  # 'device3' does not have a bootloader script and a post installation script
///       device.toml             # The device specification file for 'device3'
///   vendor2/
///     device4/
///       device.toml
///       script.sh               # Badly named bootloader script but is acceptable
/// ```
///
/// - The top-level directory contains vendor-level directories.
/// - The vendor-level directories contain device-level directories.
/// - The device-level directory is the directory containing the [device specification file]. It can also contain other device-related scripts, like post-installation script, and scripts that set up bootloaders.
/// - The vendor name and the device ID must contain only ASCII-characters, and must not contain white spaces and symbols other than hyphens and underscores. Hyphen (`-`) is preferred than underscores (`_`).
/// - Although the rules above are not enforced by the tool, you are encouraged to follow this practice. Usage outside the rules above are allowed if one has to.
/// - To save space, symbolic links of scripts are allowed.
///
/// [device specification file]: crate::device::DeviceSpec
pub struct DeviceRegistry {
	// We need to keep a list of registered devices (deserialized from
	// all or some of device.tomls from the specified registry directory).
	// We also need a lookup mechanism which makes it easier to select
	// devices.
	// To avoid unnecessary cloning (we are going to need them all, or
	// some of them, not both), the HashMap will keep the index of the
	// corresponding device in that list to save some clones.
	devices: Vec<DeviceSpec>,
	registry: HashMap<String, usize>,
}

impl DeviceRegistry {
	pub fn get_all(self) -> Result<Vec<DeviceSpec>> {
		if self.devices.is_empty() {
			bail!("Device registry contains no device.");
		}
		// self.devices gets moved
		Ok(self.devices)
	}

	pub fn get(self, str: &String) -> Result<DeviceSpec> {
		if !self.registry.contains_key(str) {
			bail!("Can't find a device with provided ID or alias '{}'", &str);
		}
		let idx_device = self.registry.get(str).unwrap();
		let device: &DeviceSpec = self
			.devices
			.get(*idx_device)
			.context("Unable to fetch device info from the registry")?;
		// Return a clone
		Ok(device.to_owned())
	}

	pub fn scan<P: AsRef<Path>>(registry_dir: P) -> Result<Self> {
		let registry_dir = registry_dir.as_ref();
		info!(
			"Scanning all devices within registry at {} ...",
			registry_dir.display()
		);
		let mut devices = Vec::new();
		let mut hashmap = HashMap::new();
		let walker = WalkDir::new(registry_dir).max_depth(4).into_iter();
		for file in walker {
			let f = file?;
			let p = f.path();
			if !p.is_file() || p.file_name().unwrap() != "device.toml" {
				continue;
			}
			let dev: DeviceSpec = DeviceSpec::from_path(p)?;
			debug!("Parsed device \"{}\"\n{:#?}", &dev.name, &dev);
			let name = dev.name.clone();
			let id = dev.id.clone();
			let aliases = dev.aliases.clone();
			if hashmap.contains_key(&id) {
				let occupant_idx = hashmap.get(&id).unwrap();
				let occupant: &DeviceSpec =
					devices.get(*occupant_idx).context(format!(
						"Can not get the device which occupies the ID '{}'",
						id
					))?;
				return Err(anyhow!("Device ID \"{}\" already exists (used by device \"{}\" ({})).\n\
						Please view the following files to decide what to do:\n- {}\n- {}",
					id, occupant.name, occupant.id, p.display(), &occupant.file_path.as_path().display())).context("Error occurred while assembling the device registry");
			}
			devices.push(dev);
			hashmap.insert(id.clone(), devices.len() - 1);
			if let Some(arr) = aliases {
				for alias in arr {
					if hashmap.contains_key(&alias) {
						let occupant_idx = hashmap.get(&alias).unwrap();
						let occupant: &DeviceSpec = devices.get(*occupant_idx).context(format!("Can not get the device which uses the alias {}", alias))?;
						return Err(anyhow!("Alias \"{}\" for device \"{}\" ({}) has been used by device \"{}\" ({})\n.\
						Please view the following files to decide what to do:\n- {}- \n{}",
							alias, name, &id, occupant.name, occupant.id, p.display(), &occupant.file_path.as_path().display()));
					}
					hashmap.insert(alias.clone(), devices.len() - 1);
				}
			}
		}
		info!(
			"Scan complete. Registry contains {} names for {} devices.",
			&hashmap.len(),
			&devices.len()
		);
		let registry = DeviceRegistry {
			devices,
			registry: hashmap,
		};
		Ok(registry)
	}

	pub fn from<P: AsRef<Path>>(path: P) -> Result<DeviceRegistry> {
		let path = path.as_ref();
		let mut registry: HashMap<String, usize> = HashMap::new();
		let devicetoml = if path.is_dir() {
			info!(
				"Trying to find a device with specified path {} ...",
				path.display().bright_cyan()
			);
			let f = PathBuf::from(path).join("device.toml");
			if !&f.exists() {
				return Err(anyhow!(
					"Specified path does not contain a device.toml file."
				));
			}
			f
		} else if path.is_file() && path.file_name().unwrap_or_default() == "device.toml" {
			info!(
				"Using specified device specification at {} ...",
				path.display().bright_cyan()
			);
			PathBuf::from(path)
		} else {
			bail!("Custom path should be either a directory that contains a device.toml or the device.toml itself.");
		};
		let device: DeviceSpec = toml::from_str(&fs::read_to_string(&devicetoml)?)?;
		let name = &device.name;
		let id = device.id.clone();
		debug!(
			"Adding device {} ({}) from {} ...",
			name,
			&id,
			&devicetoml.file_name().unwrap().to_string_lossy()
		);
		registry.insert(id, 0);
		Ok(DeviceRegistry {
			devices: vec![device],
			registry,
		})
	}

	pub fn check_validity(self) -> Result<()> {
		let mut errs = Vec::<anyhow::Error>::new();
		for d in self.devices {
			let result = d.check().context(format!(
				"Sanity check failed for device '{}' at {}:",
				&d.id,
				&d.file_path.display()
			));
			match result {
				Err(e) => {
					error!(
						"FAIL: {} ({})\n\t{}",
						&d.id,
						&d.name,
						&d.file_path.display()
					);
					errs.push(e);
				}
				Ok(_) => {
					info!(
						"PASS: {} ({})\n\t{}",
						&d.id,
						&d.name,
						&d.file_path.display()
					)
				}
			}
		}
		if errs.is_empty() {
			Ok(())
		} else {
			for e in errs {
				let mut s = String::new();
				s += &e.to_string();
				e.chain().skip(1).for_each(|c| {
					s += "\n";
					s += &c.to_string();
				});
				error!("{}", s);
			}
			bail!("Sanity check failed. Please check the output for details.")
		}
	}

	fn list_pretty(devices: Vec<DeviceSpec>) {
		// The following variables are used for formatting.
		// I prefer formatting this table by hand, since it does not bring
		// unnecessary dependencies.
		let idx_width = (devices.len().ilog10()) as usize + 1;
		println!(
			"{0} {1} {2} Vendor\n{3} Description\n{3} Aliases",
			format!("{}#", " ".repeat(idx_width - 1)),
			format!("{:<32}", "Device ID"),
			format!("{:<12}", "Arch."),
			" ".repeat(idx_width)
		);
		println!("{}", "=".repeat(80));
		let mut idx = 1;
		for device in devices.iter() {
			//  # Device ID                        Arch.       Vendor
			//    Description
			//    Aliases
			// ================================================================================
			//  1 pc-efi                           amd64       generic
			//    Standard PC (UEFI)
			//    None
			//  2 rpi-5b                           arm64       raspberrypi
			//    Raspberrt Pi 5 Model B
			//    pi5b, pi5
			println!(
				"{0} {1} {2} {3}\n{4} {5}\n{4} {6}",
				format!("{}", idx),
				format!("{:<32}", &device.id),
				format!("{:<12}", &device.arch.to_string().to_lowercase()),
				&device.vendor,
				" ".repeat(idx_width),
				&device.name,
				match &device.aliases {
					Some(aliases) => {
						if aliases.is_empty() {
							"None".to_owned()
						} else {
							aliases.join(", ")
						}
					}
					_ => "None".to_owned(),
				}
			);
			idx += 1;
			if idx > devices.len() {
				println!("\n Done listing devices.");
			} else {
				println!("{}", "-".repeat(80));
			}
		}
	}

	fn list_simple(devices: Vec<DeviceSpec>) {
		for device in devices {
			println!(
				"{:<31}\t{:<15}\t{}",
				&device.id,
				&device.arch.to_string().to_lowercase(),
				&device.name
			);
		}
	}

	pub fn list_devices(self, style: ListFormat) -> Result<()> {
		let mut devices = self.devices;
		devices.sort_by_key(|f| f.id.clone());
		info!("The list is being printned out to stdout.");
		match style {
			ListFormat::Pretty => {
				DeviceRegistry::list_pretty(devices);
			}
			ListFormat::Simple => {
				DeviceRegistry::list_simple(devices);
			} // _ => todo!(),
		}
		Ok(())
	}
}
