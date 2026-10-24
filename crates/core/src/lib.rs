pub mod addpapers;
pub mod adopt;
pub mod config;
pub mod fetcher;
pub mod items;
pub mod library;
pub mod locations;
pub mod mover;
pub mod papers;
pub mod progress;
pub mod singles;
pub mod spec;
pub mod tools;

use anyhow::Result;
use std::io::Read;

/// `n` random bytes as hex, from the operating system.
pub fn random_hex(n: usize) -> Result<String> {
    let mut buf = vec![0u8; n];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}
