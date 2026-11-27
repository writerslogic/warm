// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Read-only opens, refresh and close. See `docs/reference/concurrency.md`.

use super::*;
use crate::core::reader_snapshot::{self, READERS_DIR};

impl WarmCore {
    /// Opens an existing store for reading while another engine may hold it for writing.
    ///
    /// Takes no lock on the store and never writes to it: the engine runs over a private
    /// snapshot of the committed state under `<store>/.hms.readers/`. Queries see exactly the
    /// state committed when the snapshot was taken; [`Self::refresh`] moves to a newer one.
    /// Every mutation returns [`crate::ReadOnlyError`]. The store must already exist. Key
    /// files (`master.key`, signing and provenance keys) are read from the store, not copied.
    pub fn open_read_only(
        dimensions: u32,
        storage_path: Option<String>,
        config: Option<WarmConfig>,
    ) -> Result<Self> {
        Self::open_read_only_at(dimensions, storage_path, config, None)
    }

    /// [`Self::open_read_only`] with the snapshot kept under `snapshot_root` instead of inside
    /// the store, for a store directory the reader cannot write to.
    pub fn open_read_only_at(
        dimensions: u32,
        storage_path: Option<String>,
        config: Option<WarmConfig>,
        snapshot_root: Option<PathBuf>,
    ) -> Result<Self> {
        let store = storage_path
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(".").to_path_buf());
        let root = snapshot_root.unwrap_or_else(|| store.join(READERS_DIR));
        // Key files are not copied into the snapshot: the engine reads the store's own.
        let mut config = config.unwrap_or_default();
        let keys = [
            (
                &mut config.security.master_key_file,
                reader_snapshot::KEY_FILES[0],
            ),
            (&mut config.security.key_path, reader_snapshot::KEY_FILES[1]),
            (
                &mut config.provenance.key_path,
                reader_snapshot::KEY_FILES[2],
            ),
        ];
        for (slot, name) in keys {
            let original = store.join(name);
            if slot.is_none() && original.is_file() {
                *slot = Some(original.display().to_string());
            }
        }
        let config = Some(config);
        let snapshot = reader_snapshot::take(&store, &root)?;
        let private = snapshot.dir().display().to_string();
        Self::open_engine(dimensions, Some(private), config, true, Some(snapshot))
    }

    /// True for an engine opened with [`Self::open_read_only`].
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Whether the writer has committed since this read-only engine's snapshot. Cheap: it
    /// lists the store's files and checks the journal's frontier. Always false for a writer.
    pub fn has_newer_commits(&self) -> Result<bool> {
        match &self.reader_snapshot {
            Some(snapshot) => reader_snapshot::changed_since(snapshot.source(), snapshot.stamp()),
            None => Ok(false),
        }
    }

    /// Moves a read-only engine to the store's current committed state. Returns false and
    /// keeps the snapshot when nothing was committed since. The old state stays in use until
    /// the new one is open, so a failure leaves the engine as it was. A passphrase passed
    /// directly in the config is not retained: reopen with [`Self::open_read_only`] instead.
    pub fn refresh(&mut self) -> Result<bool> {
        let Some(snapshot) = &self.reader_snapshot else {
            return Err(anyhow::anyhow!("refresh applies to a read-only engine"));
        };
        if !self.has_newer_commits()? {
            return Ok(false);
        }
        let store = snapshot.source().display().to_string();
        let root = snapshot.root().to_path_buf();
        let fresh = Self::open_read_only_at(
            u32::try_from(self.dimensions)?,
            Some(store),
            Some(self.config.clone()),
            Some(root),
        )?;
        *self = fresh;
        Ok(true)
    }

    /// Flushes a writer's state and releases its lock, so another process can open the store
    /// for writing. A read-only engine just removes its snapshot. The engine is consumed: use
    /// after close is a compile error here, and an error in the bindings. The lock is released
    /// even if the flush fails, and the error says what was not persisted.
    pub fn close(self) -> Result<()> {
        if self.read_only {
            return Ok(());
        }
        self.flush()
    }

    pub(super) fn ensure_writable(&self) -> Result<()> {
        if self.read_only {
            return Err(crate::core::error::ReadOnlyError.into());
        }
        Ok(())
    }
}
