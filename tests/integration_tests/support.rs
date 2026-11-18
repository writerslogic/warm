// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Helpers shared by several test modules.

use std::path::Path;

use writerslogic_warm::{HistoryError, WarmCore};

pub fn open(dir: &tempfile::TempDir) -> anyhow::Result<WarmCore> {
    WarmCore::new(1024, Some(dir.path().display().to_string()), None)
}

pub fn history_error(error: anyhow::Error) -> HistoryError {
    error
        .downcast::<HistoryError>()
        .unwrap_or_else(|e| panic!("not a history error: {e:#}"))
}

/// Paths under `dir` whose bytes contain `needle`; the lock file is skipped because Windows
/// refuses to read it while the store is open and it never holds store content.
pub fn files_containing(dir: &Path, needle: &[u8], out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == ".hms.lock") {
            continue;
        }
        if path.is_dir() {
            files_containing(&path, needle, out);
        } else if std::fs::read(&path)
            .unwrap()
            .windows(needle.len())
            .any(|w| w == needle)
        {
            out.push(path.display().to_string());
        }
    }
}
