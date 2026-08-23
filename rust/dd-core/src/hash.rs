//! Byte-level hashing for the exact-duplicate pass.

use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// SHA-256 of the whole file, read in 1 MiB chunks so large files never land
/// in memory. Lowercase hex, matching the Swift implementation.
pub fn content_hash(path: &Path) -> std::io::Result<String> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; 1 << 20];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect())
}
