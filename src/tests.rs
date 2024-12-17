#![cfg(test)]
use std::str::FromStr;

use crate::{
	partition::PartitionType,
	utils::{create_sparse_file, geteuid},
};
use anyhow::{bail, Context, Result};
use log::info;
use loopdev;
use toml;
use uuid::Uuid;

#[test]
fn test_loopdev() -> Result<()> {
	env_logger::builder()
		.filter_level(log::LevelFilter::Info)
		.init();
	if unsafe { geteuid() } != 0 {
		bail!("Not being run as root user, aborting.");
	}
	create_sparse_file("/tmp/file", 512 * 1024 * 1024)?;
	let loopctl = loopdev::LoopControl::open()?;
	let loopdev = loopctl.next_free()?;
	info!(
		"Current loop device:\nPath: {}\nDevice node: {}:{}\n",
		loopdev.path().context("Jesus")?.display(),
		loopdev.major()?,
		loopdev.minor()?
	);
	loopdev.attach_file("/tmp/file")?;
	loopdev.detach()?;
	std::fs::remove_file("/tmp/file").context("Unable to remove the test sparse file")?;
	Ok(())
}

#[test]
fn test_partition_type() -> Result<()> {
	env_logger::builder()
		.filter_level(log::LevelFilter::Info)
		.init();
	let s1 = toml::to_string_pretty(&PartitionType::Basic)?;
	let s2 = toml::to_string_pretty(&PartitionType::Linux)?;
	let s3 = toml::to_string_pretty(&PartitionType::EFI)?;
	let s4 = toml::to_string_pretty(&PartitionType::Swap)?;
	let s5 = toml::to_string_pretty(&PartitionType::Uuid {
		uuid: Uuid::from_str("C12A7328-F81F-11D2-BA4B-00A0C93EC93B")?,
	})?;
	let s6 = toml::to_string_pretty(&PartitionType::Byte { byte: 0x27 })?;
	// let s5 = String::from("ok");
	info!("{}\n{}\n{}\n{}\n{}\n{}", s1, s2, s3, s4, s5, s6);
	Ok(())
}
