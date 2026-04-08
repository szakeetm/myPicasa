use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use crate::util::errors::AppError;

pub fn quick_hash(path: &Path, file_size: u64) -> Result<Vec<u8>, AppError> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();

    let mut first = vec![0_u8; 64 * 1024];
    let first_bytes = file.read(&mut first)?;
    hasher.update(&first[..first_bytes]);

    if file_size > 64 * 1024 {
        let tail_start = file_size.saturating_sub(64 * 1024);
        file.seek(SeekFrom::Start(tail_start))?;
        let mut last = vec![0_u8; 64 * 1024];
        let last_bytes = file.read(&mut last)?;
        hasher.update(&last[..last_bytes]);
    }

    hasher.update(&file_size.to_le_bytes());
    Ok(hasher.finalize().as_bytes().to_vec())
}
