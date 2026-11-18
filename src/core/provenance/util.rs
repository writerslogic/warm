// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Milliseconds since the Unix epoch; 0 if the clock reads before it. `u64` milliseconds last
/// until the year 584 million, so the narrowing cast cannot truncate.
pub(super) fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Signs `head` with `signing_key` and atomically replaces the log's `.head` file with it.
pub(super) fn write_signed_head(
    signing_key: &SigningKey,
    head: &[u8; 32],
    log_path: &Path,
) -> Result<()> {
    let head_path = log_path.with_extension("head");
    let signed = cose::sign_payload(signing_key, head).context("signed head signing failed")?;
    super::super::durable_file::atomic_write(&head_path, &signed)
        .context("signed head write failed")
}

/// Appends `bytes` to the file at `path` and syncs them to the drive. A failed write or sync
/// truncates the file back to its prior length, so later lines never chain after a torn one.
pub(super) fn append_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let prior = file.metadata()?.len();
    let written = file
        .write_all(bytes)
        .and_then(|()| file.sync_data())
        .and_then(|()| super::super::durable_file::sync_mapped_file(&file));
    if let Err(e) = written {
        if let Err(truncate) = file.set_len(prior) {
            tracing::warn!("could not drop a partial provenance log append: {truncate}");
        }
        return Err(e.into());
    }
    if prior == 0 {
        // The file may be new: its directory entry must be durable too.
        super::super::durable_file::sync_directory(
            path.parent().unwrap_or_else(|| Path::new(".")),
        )?;
    }
    Ok(())
}

pub(super) fn hex_decode(hex: &str) -> Result<[u8; 32]> {
    if hex.len() != 64 {
        return Err(anyhow::anyhow!("invalid hex hash length"));
    }
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        out[i] = u8::from_str_radix(
            std::str::from_utf8(chunk).map_err(|e| anyhow::anyhow!("hex decode: {e}"))?,
            16,
        )
        .map_err(|e| anyhow::anyhow!("hex decode: {e}"))?;
    }
    Ok(out)
}
