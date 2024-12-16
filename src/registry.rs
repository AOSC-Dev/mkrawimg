use crate::device::DeviceSpec;
use anyhow::{anyhow, bail, Context, Result};
use log::{debug, info};
use owo_colors::OwoColorize;
use std::{
	collections::HashMap,
	fs,
	path::{Path, PathBuf},
};
use walkdir::WalkDir;

/// Device Registry.
pub struct DeviceRegistry {
	// We need to keep a list of registered devices (deserialized from
	// all or some of device.tomls from the specified registry directory.
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
		Ok(())
	}
}
