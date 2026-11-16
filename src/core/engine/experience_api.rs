// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! The experience store of a [`WarmCore`]: tree-structured observations kept beside the
//! documents, journaled per step and checkpointed with the document store.

use std::sync::Arc;

use anyhow::Result;

use super::WarmCore;
use crate::core::experience::store::{self, ExperienceStore, Sealer};
use crate::core::experience::{ExperienceConfig, ExperienceError};

#[cfg(feature = "security")]
struct Encrypted(Arc<crate::core::security::EncryptionManager>);

#[cfg(feature = "security")]
impl Sealer for Encrypted {
    fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        self.0.encrypt(plaintext)
    }

    fn unseal(&self, sealed: &[u8]) -> Result<Vec<u8>> {
        self.0.decrypt(sealed)
    }
}

impl WarmCore {
    fn experience_sealer(&self) -> Option<Arc<dyn Sealer>> {
        #[cfg(feature = "security")]
        if let Some(enc) = &self.encryption {
            return Some(Arc::new(Encrypted(enc.clone())));
        }
        None
    }

    /// Opens the experience store when the directory holds one, finishing any interrupted
    /// write, checkpoint or purge.
    pub(super) fn open_experience(&self) -> Result<()> {
        if !store::exists(&self.storage_path) {
            return Ok(());
        }
        let opened = ExperienceStore::open(
            &self.storage_path,
            ExperienceConfig::default(),
            self.experience_sealer(),
        )?;
        *self.experience.write() = Some(opened);
        Ok(())
    }

    /// Runs `f` on the experience store for writing, creating it on first use.
    pub fn with_experience_mut<R>(
        &self,
        f: impl FnOnce(&mut ExperienceStore) -> R,
    ) -> std::result::Result<R, ExperienceError> {
        let mut slot = self.experience.write();
        if slot.is_none() {
            *slot = Some(ExperienceStore::create(
                &self.storage_path,
                ExperienceConfig::default(),
                self.experience_sealer(),
            )?);
        }
        Ok(f(slot.as_mut().expect("just created")))
    }

    /// Runs `f` on the experience store, or returns `None` when the store holds none.
    pub fn with_experience<R>(&self, f: impl FnOnce(&ExperienceStore) -> R) -> Option<R> {
        self.experience.read().as_ref().map(f)
    }

    /// Units the experience core holds against a tenant quota (episodes, regions, effect rows).
    pub fn experience_units(&self) -> u64 {
        self.with_experience(|s| s.quota_units()).unwrap_or(0)
    }

    /// The configured bounds on one observation.
    pub fn experience_limits(&self) -> ExperienceConfig {
        ExperienceConfig::default()
    }

    /// Writes the experience snapshot and empties its journal when anything changed since the
    /// last one. Runs with every document checkpoint and flush.
    pub fn experience_checkpoint(&self) -> Result<()> {
        if self
            .experience
            .read()
            .as_ref()
            .is_none_or(|store| !store.is_dirty())
        {
            return Ok(());
        }
        if let Some(store) = self.experience.write().as_mut() {
            if store.is_dirty() {
                store.checkpoint()?;
            }
        }
        Ok(())
    }
}
