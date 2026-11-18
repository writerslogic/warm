// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl WarmCore {
    pub(super) fn shard_meta_path(base: &Path) -> PathBuf {
        base.join("shard_meta.json")
    }

    /// Read the persisted shard count written by a prior auto-shard, if any. An unreadable or
    /// malformed file reads as absent; `warm-admin verify` reports it as `shard-meta-invalid`.
    pub(super) fn load_shard_meta(base: &Path) -> Option<usize> {
        let data = std::fs::read_to_string(Self::shard_meta_path(base)).ok()?;
        let value: serde_json::Value = serde_json::from_str(&data).ok()?;
        value.get("shard_count")?.as_u64().map(|n| n as usize)
    }

    pub(super) fn save_shard_meta(&self, shard_count: usize) -> Result<()> {
        let json = serde_json::json!({ "shard_count": shard_count });
        super::super::durable_file::atomic_write(
            &Self::shard_meta_path(&self.storage_path),
            &serde_json::to_vec(&json)?,
        )?;
        Ok(())
    }

    /// Per-shard NSG index path. Single-shard stores keep the legacy filename so
    /// existing on-disk indices continue to load.
    pub(super) fn nsg_index_path(&self, shard_idx: usize, multi: bool) -> PathBuf {
        if multi {
            self.storage_path.join(format!("nsg_index_{shard_idx}.bin"))
        } else {
            self.storage_path.join("nsg_index.bin")
        }
    }

    pub(super) fn ivf_index_path(&self, shard_idx: usize, multi: bool) -> PathBuf {
        if multi {
            self.storage_path.join(format!("ivf_index_{shard_idx}.bin"))
        } else {
            self.storage_path.join("ivf_index.bin")
        }
    }

    pub(super) fn load_indices(&self) -> Result<()> {
        let metadata = self.storage_path.join("indices.json");
        let Ok(bytes) = std::fs::read(metadata) else {
            return Ok(());
        };
        let saved: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(%error, "index metadata invalid; using rebuilt inverted index");
                return Ok(());
            }
        };
        if saved["encoding"] != index_cache::ENCODING {
            return Ok(());
        }
        if saved["checkpoint"] != serde_json::to_value(self.arena.checkpoint())? {
            tracing::info!("stale ANN checkpoint; using rebuilt inverted index");
            return Ok(());
        }
        let shards = self.shards.read();
        let multi = shards.shard_count() > 1;
        if saved["shardCount"] != shards.shard_count() {
            return Ok(());
        }
        shards.try_for_each_shard_indexed(|i, shard| {
            let read_cache = |path: &Path, expected_crc: &serde_json::Value| -> Result<Vec<u8>> {
                let raw = index_cache::read(path)?;
                anyhow::ensure!(
                    expected_crc.as_u64() == Some(crc32fast::hash(&raw) as u64),
                    "index cache checksum mismatch"
                );
                self.maybe_decrypt(raw)
            };
            let nsg_path = self.nsg_index_path(i, multi);
            if nsg_path.exists() && saved["shards"][i]["nsg"] == true {
                let result = (|| -> Result<super::super::nsg::NSGIndex> {
                    let data = read_cache(&nsg_path, &saved["shards"][i]["nsgCrc"])?;
                    let nsg = index_cache::decode(&data)?;
                    index_cache::validate_nsg(&nsg, shard, self.dimensions)?;
                    Ok(nsg)
                })();
                match result {
                    Ok(nsg) => *shard.nsg.write() = Some(nsg),
                    Err(error) => tracing::warn!(%error, shard = i, "discarding invalid NSG cache"),
                }
            }

            let ivf_path = self.ivf_index_path(i, multi);
            if ivf_path.exists() && saved["shards"][i]["ivf"] == true {
                let result = (|| -> Result<IVFIndex> {
                    let data = read_cache(&ivf_path, &saved["shards"][i]["ivfCrc"])?;
                    let mut ivf: IVFIndex = index_cache::decode(&data)?;
                    index_cache::validate_ivf(&ivf, self.dimensions)?;
                    ivf.lists = Some(super::super::ivf::inverted_list::InvertedLists::new());

                    let vectors = shard.vectors.read();
                    let registry = shard.registry.read();
                    for id in registry.iter() {
                        if let Some(vec) = vectors.get(id) {
                            ivf.insert(id, vec)?;
                        }
                    }
                    Ok(ivf)
                })();
                match result {
                    Ok(ivf) => *shard.ivf.write() = Some(ivf),
                    Err(error) => tracing::warn!(%error, shard = i, "discarding invalid IVF cache"),
                }
            }
            Ok(())
        })
    }

    /// Persist every shard's trained NSG/IVF index to disk. Applies to both
    /// single- and multi-shard stores; multi-shard indices are written to
    /// per-shard files so they reload after a restart.
    pub(in super::super) fn persist_indices(&self, shards: &ShardSet) -> Result<()> {
        // The checkpoint is read before the encoding: an append in between leaves the
        // metadata behind the arena, which the loader treats as stale (see `encode_indices`).
        let checkpoint = self.arena.checkpoint();
        let encoded = self.encode_indices(shards, checkpoint)?;
        self.write_indices(encoded)
    }

    /// Serializes every shard's trained NSG/IVF index with the metadata that binds the files
    /// to `checkpoint`. The caller holds `shards` while the encoding reads them; the files
    /// are written by [`Self::write_indices`] without any lock. A checkpoint read before a
    /// mutation that the encoded indices already include only makes the loader rebuild them;
    /// a checkpoint read after a mutation they lack would make it trust stale indices, so the
    /// checkpoint must be read while no writer can be in flight (under a writer lease or an
    /// exclusive publication).
    pub(in super::super) fn encode_indices(
        &self,
        shards: &ShardSet,
        checkpoint: (String, usize),
    ) -> Result<EncodedIndices> {
        let multi = shards.shard_count() > 1;
        let mut files = Vec::new();
        let mut states = Vec::new();
        shards.try_for_each_shard_indexed(|i, shard| {
            let mut nsg_crc = None;
            let mut ivf_crc = None;
            if let Some(ref nsg) = *shard.nsg.read() {
                if let Some(data) = index_cache::encode(nsg)? {
                    let data = self.maybe_encrypt(&data)?.into_owned();
                    nsg_crc = Some(crc32fast::hash(&data));
                    files.push((self.nsg_index_path(i, multi), data));
                }
            }
            if let Some(ref ivf) = *shard.ivf.read() {
                if let Some(data) = index_cache::encode(ivf)? {
                    let data = self.maybe_encrypt(&data)?.into_owned();
                    ivf_crc = Some(crc32fast::hash(&data));
                    files.push((self.ivf_index_path(i, multi), data));
                }
            }
            states
                .push(serde_json::json!({"nsg": nsg_crc.is_some(), "ivf": ivf_crc.is_some(), "nsgCrc": nsg_crc, "ivfCrc": ivf_crc}));
            Ok(())
        })?;
        let metadata = serde_json::to_vec(
            &serde_json::json!({"encoding": index_cache::ENCODING, "checkpoint": checkpoint, "shardCount": shards.shard_count(), "shards": states}),
        )?;
        Ok(EncodedIndices { files, metadata })
    }

    /// Writes the encoded index files and metadata, unless this process already wrote
    /// byte-identical metadata (same checkpoint, same file checksums), which makes the
    /// write a no-op.
    pub(in super::super) fn write_indices(&self, encoded: EncodedIndices) -> Result<()> {
        if *self.persisted_indices.lock() == encoded.metadata {
            return Ok(());
        }
        for (path, data) in &encoded.files {
            super::super::durable_file::atomic_write(path, data)?;
        }
        super::super::durable_file::atomic_write(
            &self.storage_path.join("indices.json"),
            &encoded.metadata,
        )?;
        *self.persisted_indices.lock() = encoded.metadata;
        Ok(())
    }
}
