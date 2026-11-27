// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl WarmCore {
    /// Returns true if the IVF index has been trained.
    pub fn ivf_trained(&self) -> bool {
        self.shards.read().ivf_trained()
    }

    /// Train the IVF index on current vectors. Persists the index to disk.
    pub fn train_ivf(&self) -> Result<()> {
        self.train_shard_indices(
            |vectors, ids| IVFIndex::train(vectors, ids, self.dimensions, &self.config.ivf),
            |shard, index| *shard.ivf.write() = index,
        )
    }

    /// Returns true if the NSG graph index has been trained.
    pub fn nsg_trained(&self) -> bool {
        self.shards.read().nsg_trained()
    }

    /// Train the NSG graph index on current vectors. Persists the index to disk.
    pub fn train_nsg(&self) -> Result<()> {
        self.train_shard_indices(
            |vectors, ids| super::super::nsg::training::train(vectors, ids, &self.config.nsg),
            |shard, index| *shard.nsg.write() = index,
        )
    }

    /// Trains one index per shard from a snapshot taken under the read gate, without holding
    /// the gate, then installs and persists them only if no mutation landed meanwhile.
    pub(super) fn train_shard_indices<I>(
        &self,
        train: impl Fn(&[EntangledHVec], &[String]) -> Result<I>,
        install: impl Fn(&shard::Shard, Option<I>),
    ) -> Result<()> {
        let (revision, snapshots) = {
            let _transaction = self.mutation_gate.read();
            let shards = self.shards.read();
            let mut snapshots = Vec::new();
            shards.for_each_shard(|shard| snapshots.push(shard.load_all_vectors()));
            (
                self.revision.load(std::sync::atomic::Ordering::Acquire),
                snapshots,
            )
        };
        let mut indices = Vec::with_capacity(snapshots.len());
        for (ids, vectors) in snapshots {
            indices.push(if ids.is_empty() {
                None
            } else {
                Some(train(&vectors, &ids)?)
            });
        }
        let _transaction = self.mutation_gate.write();
        anyhow::ensure!(
            revision == self.revision.load(std::sync::atomic::Ordering::Acquire),
            "store changed during training; retry maintenance"
        );
        let shards = self.shards.read();
        shards.try_for_each_shard_indexed(|i, shard| {
            install(shard, indices[i].take());
            Ok(())
        })?;
        self.persist_indices(&shards)
    }

    pub(super) fn arena_write(&self, data: &[u8]) -> Result<usize> {
        self.ensure_writable()?;
        let payload = self.maybe_encrypt(data)?;
        self.arena.write_slice(&payload)
    }

    #[cfg(test)] // replay reads and decrypts in two steps to tell a bad frame from a wrong passphrase
    pub(super) fn arena_read_frame(&self, offset: usize) -> Result<(Vec<u8>, u32)> {
        let (data, version) = self.arena.read_frame(offset)?;
        Ok((self.maybe_decrypt(data)?, version))
    }

    pub(super) fn sign_fn(&self) -> Option<SignFn<'_>> {
        #[cfg(feature = "security")]
        {
            self.signing
                .as_ref()
                .map(|s| Box::new(move |data: &[u8]| s.sign(data)) as SignFn<'_>)
        }
        #[cfg(not(feature = "security"))]
        {
            None
        }
    }
}
