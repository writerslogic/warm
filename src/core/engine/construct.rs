// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl WarmCore {
    /// Create a new WARM instance. If `storage_path` is None, uses the current directory.
    pub fn new(
        dimensions: u32,
        storage_path: Option<String>,
        config: Option<WarmConfig>,
    ) -> Result<Self> {
        const MAX_DIMENSIONS: u32 = 1_000_000;
        if dimensions == 0 || dimensions > MAX_DIMENSIONS {
            return Err(anyhow::anyhow!(
                "dimensions must be between 1 and {} (got {})",
                MAX_DIMENSIONS,
                dimensions
            ));
        }
        let dim = dimensions as usize;
        let config = config.unwrap_or_default();
        #[cfg(feature = "local-embed")]
        let (config, text_embedder) = embed_api::prepare(config)?;
        config.validate()?;
        anyhow::ensure!(
            EntangledHVec::dp_scale(dim, config.privacy.epsilon).is_some(),
            "privacy epsilon is too small for the configured dimensions"
        );
        #[cfg(feature = "security")]
        let mut config = config;

        let base_path = storage_path
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(".").to_path_buf());
        if !base_path.exists() {
            std::fs::create_dir_all(&base_path)?;
        }

        let store_lock = StoreLock::acquire(&base_path)?;
        let resolved = super::super::schema::resolve_protection(&base_path, &config)?;
        #[cfg(feature = "security")]
        {
            config.security.encryption_enabled =
                resolved.protection != super::super::schema::Protection::Plaintext;
        }
        let (schema, analyzer) =
            super::super::schema::validate_store_resolved(&base_path, dim, &config, resolved)?;
        let dense = dense_vectors::DenseVectors::open(&base_path)?;
        let base_path_sparse = base_path.clone();

        let arena = PersistentArena::new(base_path.join("vectors_data.bin"))?;

        let audit = if config.security.audit_enabled {
            Some(AuditLog::new(&base_path)?)
        } else {
            None
        };

        let audit_effects = if schema >= super::super::format::AUDIT_EFFECTS_SCHEMA_VERSION {
            Some(super::super::audit_effects::AuditEffectStream::open_existing(&base_path)?)
        } else {
            None
        };

        #[cfg(feature = "security")]
        let signing = if config.security.signing_enabled {
            let key_path = config
                .security
                .key_path
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or_else(|| base_path.join("hms_signing.key"));
            Some(super::super::security::SigningManager::new(&key_path)?)
        } else {
            None
        };

        #[cfg(feature = "security")]
        let encryption = if resolved.protection == super::super::schema::Protection::MasterKey {
            Some(std::sync::Arc::new(
                super::super::security::EncryptionManager::open_master(
                    &base_path,
                    config.security.master_key_file.as_deref(),
                    resolved.new_store,
                )?,
            ))
        } else if resolved.protection == super::super::schema::Protection::Passphrase {
            use zeroize::Zeroize;
            let mut passphrase = config
                .security
                .encryption_passphrase
                .take()
                .or_else(|| {
                    config
                        .security
                        .encryption_passphrase_env
                        .as_deref()
                        .and_then(|name| crate::compatibility::var(name).ok())
                })
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "encryption requires encryption_passphrase or encryption_passphrase_env"
                    )
                })?;
            let manager = super::super::security::EncryptionManager::new(&passphrase, &base_path);
            passphrase.zeroize();
            Some(std::sync::Arc::new(manager?))
        } else {
            None
        };

        #[cfg(feature = "provenance")]
        let provenance = if config.provenance.enabled {
            let key_path = config
                .provenance
                .key_path
                .as_ref()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| base_path.join("hms_provenance.key"));
            let mgr =
                super::super::provenance::ProvenanceManager::new(&key_path, Some(&base_path))?;
            let mgr = if let Some(ref url) = config.provenance.status_list_url {
                mgr.with_status_list_url(url.clone())?
            } else {
                mgr
            };
            #[cfg(feature = "provenance-scitt")]
            let mgr = if let Some(ref endpoint) = config.provenance.scitt_endpoint {
                mgr.with_scitt_endpoint(endpoint.clone())
            } else {
                mgr
            };
            Some(mgr)
        } else {
            None
        };

        let shard_set = if config.shard.enabled && config.shard.shard_count > 1 {
            ShardSet::Multi(ShardManager::new(config.shard.shard_count, dim))
        } else if let Some(n) = Self::load_shard_meta(&base_path).filter(|&n| n > 1) {
            // A store that auto-sharded in a previous run persists its shard
            // count so the topology (and per-shard indices) reload correctly.
            ShardSet::Multi(ShardManager::new(n, dim))
        } else {
            ShardSet::Single(Box::new(shard::Shard::new(dim)))
        };

        let (atom_mem, comp_mem, tri_store, role_reg, rule_st, decomp, adm) =
            if config.meaning.enabled {
                let mc = &config.meaning;
                (
                    Some(Arc::new(AtomMemory::new(dim, mc.idf_clip_factor))),
                    Some(Arc::new(CompositeMemory::new(dim, mc.idf_clip_factor))),
                    Some(Arc::new(TripleStore::new())),
                    Some(RoleRegistry::new(dim)),
                    Some(RuleStore::new()),
                    Some(Decomposer::new()),
                    Some(AdmissionControl::new(mc.algebraic_max_fanout)),
                )
            } else {
                (None, None, None, None, None, None, None)
            };

        let purge = Arc::new(purge::PurgeLedger::open(&base_path)?);
        let core = Self {
            config: config.clone(),
            mutation_gate: mutation_gate::MutationGate::default(),
            stem_memos: Default::default(),
            compaction: crate::core::sync::Mutex::new(()),
            history_floor: RwLock::new((String::new(), 0)),
            history_cache: Default::default(),
            persisted_indices: crate::core::sync::Mutex::new(Vec::new()),
            revision: std::sync::atomic::AtomicU64::new(
                super::super::documents::opaque_counter_start(),
            ),
            vector_record_instance: rand::random(),
            journal: journal::JournalFormat::for_schema(schema),
            schema,
            analyzer,
            arena,
            dimensions: dim,
            storage_path: base_path,
            shards: RwLock::new(shard_set),
            graph: RelationStore::new(),
            documents: RwLock::new(super::super::documents::DocumentStore::opened()),
            image: image::ImageState::default(),
            vector_index: super::super::documents::VectorIndexSlot::default(),
            dense,
            sparse: crate::core::index::sparse_store::SparseStore::open(&base_path_sparse)?,
            #[cfg(feature = "local-models")]
            model_stages: RwLock::new(None),
            #[cfg(feature = "local-embed")]
            text_embedder: RwLock::new(text_embedder),
            derived: RwLock::new(std::collections::BTreeMap::new()),
            atom_memory: atom_mem,
            composite_memory: comp_mem,
            triple_store: tri_store,
            role_registry: role_reg,
            rule_store: rule_st,
            decomposer: decomp,
            admission: adm,
            cognition_loop: crate::core::sync::Mutex::new(None),
            goal_store: if config.meaning.enabled {
                Some(GoalStore::new())
            } else {
                None
            },
            self_modifier: if config.meaning.enabled {
                Some(SelfModifier::new())
            } else {
                None
            },
            audit,
            audit_effects,
            audit_pending: RwLock::default(),
            audit_delivery: mutation_gate::MutationGate::default(),
            #[cfg(feature = "security")]
            signing,
            #[cfg(feature = "security")]
            encryption,
            #[cfg(feature = "provenance")]
            agents: parking_lot::RwLock::new(
                super::super::provenance::access::AgentRegistry::default(),
            ),
            #[cfg(feature = "provenance")]
            provenance,
            #[cfg(feature = "experimental")]
            connection_graph: crate::core::sync::Mutex::new(None),
            #[cfg(feature = "experimental")]
            phase_graph: crate::core::sync::Mutex::new(None),
            purge,
            scrub_gate: crate::core::sync::Mutex::new(()),
            experience: RwLock::new(None),
            _store_lock: store_lock,
        };

        let text_path = core.storage_path.join("documents.text");
        // A purge interrupted before renaming its canonical segment leaves a copy of the live
        // text, which a later purge would not reach.
        let staging = super::super::documents::canonical_staging_path(&text_path);
        if staging.exists() {
            super::super::durable_file::scrub_remove_file(&staging)?;
        }
        match core.open_checkpoint()? {
            image::Opened::Served => {}
            image::Opened::Overlaid => core.load_from_log()?,
            image::Opened::Absent => {
                core.documents.write().attach_text_segment(
                    &text_path,
                    #[cfg(feature = "security")]
                    core.encryption.clone(),
                )?;
                core.load_from_log()?;
            }
        }
        *core.history_floor.write() = core.initial_history_floor();
        core.load_indices()?;
        {
            let shards = core.shards.read();
            shards.try_for_each_shard(|s| s.rebuild_inverted_index(dim))?;
        }
        // Only a completely opened engine may write an image when it closes.
        core.flush_audit()?;
        core.open_experience()?;
        core.mark_opened();
        // A purge that died between its delete commit and the end of its compaction left the
        // deleted text in the journal; finish it before the store is handed out.
        if core.purge_pending() {
            if let Err(error) = core.compact_scrubbing() {
                // A full disk (or any failure) must not make the store unopenable: nothing
                // acknowledged is lost, the deletes are committed, and the ticket stays on disk
                // so a later open (or `compact_scrubbing`) finishes the purge. The store reports
                // it as pending (`purge_status`), never as purged.
                tracing::warn!(
                    error = %format!("{error:#}"),
                    "an interrupted purge could not be completed; it stays pending"
                );
            }
        }

        Ok(core)
    }
}
