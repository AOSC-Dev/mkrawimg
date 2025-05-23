use std::{
	fs::{File, create_dir_all},
	io::Write,
	path::{Path, PathBuf},
};

use anyhow::{Result, anyhow, bail};
use log::info;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

/// Represents a topic. Serializes to /var/lib/atm/state.
#[derive(Deserialize, Serialize, Clone)]
// arch and draft are not used
#[allow(dead_code)]
pub struct Topic {
	/// Topic name.
	name: String,
	/// Topic description.
	description: Option<String>,
	/// Date of the launch - as time64_t.
	date: u64,
	/// Update date of this topic - as time_t.
	update_date: u64,
	/// Available archs in this topic.
	#[serde(skip_serializing)]
	arch: Vec<String>,
	/// Affected packages in this topic.
	packages: Vec<String>,
	/// Whether the corresponding PR is a draft.
	#[serde(skip_serializing)]
	draft: bool,
}

const ATM_STATE: &str = "var/lib/atm/state";
const ATM_LIST: &str = "etc/apt/sources.list.d/atm.list";
const DEFAULT_MIRROR: &str = "https://repo.aosc.io/debs";
const TOPIC_MANIFEST_URL: &str = "https://repo.aosc.io/debs/manifest/topics.json";

pub fn fetch_topics() -> Result<Vec<Topic>> {
	info!("Fetching topics manifest ...");
	let client = Client::builder()
		.user_agent("Wget/1.20.3 (linux-gnu)")
		.build()?;
	let response = client.get(TOPIC_MANIFEST_URL).send()?;
	response.error_for_status_ref()?;
	let topics: Vec<Topic> = serde_json::from_str(&response.text()?)?;
	Ok(topics)
}

pub fn filter_topics(specified: &[String], all: Vec<Topic>) -> Result<Vec<Topic>> {
	info!("Checking availability of specified topics ...");
	let mut filtered = Vec::<Topic>::new();
	let mut specified = specified.to_owned();
	specified.sort();
	all.iter().for_each(|t| {
		if specified.contains(&t.name) {
			filtered.push(t.clone());
		}
	});
	if filtered.is_empty() {
		let all_names = &all.into_iter().map(|x| x.name).collect::<Vec<_>>();
		bail!(
			"{}\n{}\n{:#?}",
			"None of specified topic names exist. Please check your input.",
			"All available topics:",
			all_names
		);
	}
	Ok(filtered)
}

pub fn save_topics(sysroot: &Path, topics: &Vec<Topic>) -> Result<()> {
	info!("Saving topic sources and ATM state ...");
	// Prepare paths
	let mut atm_list_path = PathBuf::from(sysroot);
	atm_list_path.push(ATM_LIST);
	let mut atm_state_path = PathBuf::from(sysroot);
	atm_state_path.push(ATM_STATE);
	let atm_list_parent = atm_list_path.parent().ok_or(anyhow!(
		"Failed to get parent path of {:#?}",
		&atm_list_path
	))?;
	let atm_state_parent = atm_state_path.parent().ok_or(anyhow!(
		"Failed to get parent path of {:#?}",
		&atm_state_path
	))?;
	create_dir_all(atm_list_parent)?;
	create_dir_all(atm_state_parent)?;

	// Prepare APT sources
	let topic_sources: Vec<String> = topics
		.iter()
		.map(|x| format!("deb {} {} main", DEFAULT_MIRROR, x.name.clone()))
		.collect();

	// Save atm.list
	info!("Saving topic sources ...");
	let content = topic_sources
		.into_iter()
		.map(|x| (x + "\n"))
		.collect::<String>();
	let buf = content.as_bytes();
	let mut writer = File::create(atm_list_path)?;
	writer.write_all(buf)?;
	writer.sync_all()?;

	// Save /var/lib/atm/state
	info!("Saving ATM state file ...");
	let writer = File::create(atm_state_path)?;
	serde_json::to_writer(writer, &topics)?;
	info!("saved {} topics into the target system.", topics.len());
	Ok(())
}

#[test]
fn test_fetch_topics() -> Result<()> {
	let topics = fetch_topics()?;
	println!("Fetched topics:");
	for topic in topics {
		println!(
			"Name: {}\nDescription: {}",
			topic.name,
			topic.description.unwrap_or(String::from("No description"))
		);
	}
	Ok(())
}

#[test]
fn test_save_topics() -> Result<()> {
	let topics = fetch_topics()?;
	save_topics(&PathBuf::from("/tmp/aoscbootstrap"), &topics)
}

#[test]
fn test_save_empty_topics() -> Result<()> {
	let topics = Vec::<Topic>::new();
	save_topics(&PathBuf::from("/tmp/aoscbootstrap"), &topics)
}
