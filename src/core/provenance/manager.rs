// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl ProvenanceManager {
    pub fn new(key_path: &Path, storage_path: Option<&Path>) -> Result<Self> {
        let signing_key = super::super::security::SigningManager::new(key_path)?.into_signing_key();
        let issuer_did = did::did_key_from_ed25519(&signing_key.verifying_key().to_bytes());
        let log_path = storage_path.map(|p| p.join("provenance_log.jsonl"));
        let (records, chain_head, mut clock_init) = Self::load_log(log_path.as_deref())?;
        let revocation_bitstring = Self::load_revocation_bitstring(log_path.as_deref())?;
        if let Some((byte, bits)) = revocation_bitstring
            .iter()
            .enumerate()
            .rev()
            .find(|(_, b)| **b != 0)
        {
            let bit = 7 - bits.leading_zeros() as u64;
            clock_init = clock_init.max(byte as u64 * 8 + bit + 1);
        }
        let mut kel = match storage_path {
            Some(p) => match keri::KeyEventLog::with_path(&p.join("kel.json")) {
                Ok(kel) => kel,
                // `persist` replaces the file atomically, but versions before it did not, so a
                // crash there can have left a torn file; refusing to open would turn that crash
                // into an outage.
                Err(e) => {
                    tracing::warn!("KEL unreadable, starting an in-memory log: {e}");
                    keri::KeyEventLog::default()
                }
            },
            None => keri::KeyEventLog::new(),
        };
        if kel.events().is_empty() {
            if let Err(e) = kel.inception(&signing_key, None) {
                tracing::warn!("KEL inception failed: {e}");
            }
        }

        Ok(Self {
            signing_key,
            status_list_url: vc::status_list_credential_url(&issuer_did),
            issuer_did,
            records: RwLock::new(records),
            chain_head: Mutex::new(chain_head),
            logical_clock: std::sync::atomic::AtomicU64::new(clock_init),
            log_path: log_path.clone(),
            revocation_bitstring: RwLock::new(revocation_bitstring),
            kel: RwLock::new(kel),
            #[cfg(feature = "provenance-scitt")]
            scitt_endpoint: None,
        })
    }

    pub fn from_signing_key(signing_key: SigningKey) -> Self {
        let issuer_did = did::did_key_from_ed25519(&signing_key.verifying_key().to_bytes());
        let mut kel = keri::KeyEventLog::new();
        if let Err(e) = kel.inception(&signing_key, None) {
            tracing::warn!("KEL inception failed: {e}");
        }
        Self {
            signing_key,
            status_list_url: vc::status_list_credential_url(&issuer_did),
            issuer_did,
            records: RwLock::new(crate::core::hash::FastMap::default()),
            chain_head: Mutex::new([0u8; 32]),
            logical_clock: std::sync::atomic::AtomicU64::new(0),
            log_path: None,
            revocation_bitstring: RwLock::new(Vec::new()),
            kel: RwLock::new(kel),
            #[cfg(feature = "provenance-scitt")]
            scitt_endpoint: None,
        }
    }

    pub fn with_status_list_url(mut self, url: String) -> Result<Self> {
        status_list::validate_url(&url)?;
        let legacy = vc::status_list_credential_url(&self.issuer_did);
        for record in self.records.read().values() {
            if let Some(ref json) = record.vc_json {
                let credential: vc::FactCredential = serde_json::from_str(json)?;
                if let Some(status) = credential.credential_status {
                    anyhow::ensure!(
                        status.status_list_credential == url
                            || status.status_list_credential == legacy,
                        "configured status list URL differs from an issued credential"
                    );
                }
            }
        }
        self.status_list_url = url;
        Ok(self)
    }

    pub(super) fn reserve_sequences(&self, count: usize) -> Result<u64> {
        use std::sync::atomic::Ordering::SeqCst;

        let count = u64::try_from(count)?;
        let mut next = self.logical_clock.load(SeqCst);
        loop {
            let end = next
                .checked_add(count)
                .filter(|end| *end <= status_list::MAX_BYTES as u64 * 8)
                .ok_or_else(|| anyhow::anyhow!("credential status sequence space is exhausted"))?;
            match self
                .logical_clock
                .compare_exchange_weak(next, end, SeqCst, SeqCst)
            {
                Ok(_) => return Ok(next),
                Err(current) => next = current,
            }
        }
    }

    #[cfg(feature = "provenance-scitt")]
    pub fn with_scitt_endpoint(mut self, endpoint: String) -> Self {
        self.scitt_endpoint = Some(endpoint);
        self
    }

    pub fn issuer_did(&self) -> &str {
        &self.issuer_did
    }

    /// Appends `record` to the log, when there is one, then makes it the live record for
    /// `fact_id`. A failed append leaves both unchanged; a signed-head failure returns a
    /// `CommittedMutationError` after publishing the durable record.
    pub fn store_record(&self, fact_id: &str, record: ProvenanceRecord) -> Result<()> {
        let next = record
            .sequence
            .checked_add(1)
            .filter(|end| *end <= status_list::MAX_BYTES as u64 * 8)
            .ok_or_else(|| anyhow::anyhow!("credential status sequence space is exhausted"))?;
        let append = match self.log_path {
            Some(ref path) => {
                Some(
                    self.append_chained(path, 1, &self.signing_key, |_, prev_hash| {
                        serde_json::to_string(&LogEntryRef {
                            prev_hash,
                            fact_id,
                            record: &record,
                        })
                    })?,
                )
            }
            None => None,
        };
        self.logical_clock
            .fetch_max(next, std::sync::atomic::Ordering::SeqCst);
        self.records.write().insert(fact_id.to_string(), record);
        if let Some(append) = append {
            append.finish()?;
        }
        Ok(())
    }

    /// Appends `count` lines to the hash-chained log at `path`, line `i` built by `line_for`
    /// from the hex of its predecessor's hash, then advances the head once and signs it with
    /// `head_key`. The head stays locked throughout so concurrent writers cannot chain two
    /// lines to the same predecessor, and it advances only after the lines are written and
    /// synced: a failed append returns the error with the head unchanged and no torn line left
    /// behind. On success the head lock is returned: callers update the live records before
    /// releasing it, so those updates happen in log order and match a replay of the log.
    pub(super) fn append_chained(
        &self,
        path: &Path,
        count: usize,
        head_key: &SigningKey,
        mut line_for: impl FnMut(usize, String) -> serde_json::Result<String>,
    ) -> Result<ChainedAppend<'_>> {
        let mut head = self.chain_head.lock();
        let mut new_head = *head;
        let mut buf = String::new();
        for i in 0..count {
            let line = line_for(i, hex_encode(&new_head))?;
            new_head = Sha256::digest(line.as_bytes()).into();
            buf.push_str(&line);
            buf.push('\n');
        }
        append_synced(path, buf.as_bytes())
            .map_err(|e| anyhow::anyhow!("provenance log write failed: {e}"))?;
        // The lines are durable, so the head advances even if re-signing it fails below;
        // `verify_log_integrity` reports a stale signed head until the next append.
        *head = new_head;
        let anchor = write_signed_head(head_key, &new_head, path);
        Ok(ChainedAppend {
            _head: head,
            anchor,
        })
    }

    pub fn get_record(&self, fact_id: &str) -> Option<ProvenanceRecord> {
        self.records.read().get(fact_id).cloned()
    }

    pub fn record_count(&self) -> usize {
        self.records.read().len()
    }

    pub fn record_deletion(&self, fact_id: &str, reason: Option<&str>) -> Result<DeletionRecord> {
        let timestamp_ms = unix_millis();

        let mut claim_payload = Vec::new();
        let cbor_claim = ciborium::Value::Map(vec![
            (
                ciborium::Value::Text("action".to_string()),
                ciborium::Value::Text("delete".to_string()),
            ),
            (
                ciborium::Value::Text("fact_id".to_string()),
                ciborium::Value::Text(fact_id.to_string()),
            ),
            (
                ciborium::Value::Text("timestamp_ms".to_string()),
                ciborium::Value::Integer(timestamp_ms.into()),
            ),
        ]);
        ciborium::into_writer(&cbor_claim, &mut claim_payload)
            .map_err(|e| anyhow::anyhow!("CBOR encoding failed: {e}"))?;

        let cose_envelope = cose::sign_payload(&self.signing_key, &claim_payload)?;

        let record = DeletionRecord {
            fact_id: fact_id.to_string(),
            deleted_by: Some(self.issuer_did.clone()),
            timestamp_ms,
            reason: reason.map(String::from),
            cose_envelope: Some(cose_envelope),
        };

        let append = match self.log_path {
            Some(ref path) => {
                Some(
                    self.append_chained(path, 1, &self.signing_key, |_, prev_hash| {
                        serde_json::to_string(&serde_json::json!({
                            "prev_hash": prev_hash,
                            "action": "delete",
                            "fact_id": fact_id,
                            "deletion": record,
                        }))
                    })?,
                )
            }
            None => None,
        };
        self.records.write().remove(fact_id);
        if let Some(append) = append {
            append.finish()?;
        }

        Ok(record)
    }

    pub fn rotate_key(&mut self, new_key_path: &Path) -> Result<String> {
        let old_did = self.issuer_did.clone();
        let new_signing_key =
            super::super::security::SigningManager::new(new_key_path)?.into_signing_key();
        let new_did = did::did_key_from_ed25519(&new_signing_key.verifying_key().to_bytes());

        let mut rotation_payload = Vec::new();
        let cbor = ciborium::Value::Map(vec![
            (
                ciborium::Value::Text("action".to_string()),
                ciborium::Value::Text("key_rotation".to_string()),
            ),
            (
                ciborium::Value::Text("old_did".to_string()),
                ciborium::Value::Text(old_did.clone()),
            ),
            (
                ciborium::Value::Text("new_did".to_string()),
                ciborium::Value::Text(new_did.clone()),
            ),
        ]);
        ciborium::into_writer(&cbor, &mut rotation_payload)
            .map_err(|e| anyhow::anyhow!("CBOR encoding failed: {e}"))?;
        let rotation_envelope = cose::sign_payload(&self.signing_key, &rotation_payload)?;

        // The old key signs the rotation line; the new key signs the head after it, since the
        // manager verifies the head with its current key, here and after a reopen with the new
        // key file.
        let anchor = if let Some(ref path) = self.log_path {
            self.append_chained(path, 1, &new_signing_key, |_, prev_hash| {
                serde_json::to_string(&serde_json::json!({
                    "prev_hash": prev_hash,
                    "action": "key_rotation",
                    "old_did": old_did,
                    "new_did": new_did,
                    "cose_envelope": rotation_envelope,
                }))
            })?
            .finish()
        } else {
            Ok(())
        };

        self.signing_key = new_signing_key;
        self.issuer_did = new_did.clone();
        anchor?;
        Ok(new_did)
    }

    pub fn verify_log_integrity(&self) -> Result<bool> {
        Ok(self.log_integrity()? == LogIntegrity::Verified)
    }

    /// Missing evidence never verifies; filesystem failures remain errors. Replay tolerance
    /// is deliberately separate from verification of the complete signed history.
    pub fn log_integrity(&self) -> Result<LogIntegrity> {
        let Some(ref path) = self.log_path else {
            return Ok(LogIntegrity::Volatile);
        };
        let known_head = *self.chain_head.lock();
        let signed_bytes = match std::fs::read(path.with_extension("head")) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).context("failed to read provenance signed head"),
        };
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(if signed_bytes.is_some() || known_head != [0; 32] {
                    LogIntegrity::Corrupt
                } else {
                    LogIntegrity::Empty
                });
            }
            Err(e) => return Err(e).context("failed to open provenance log"),
        };
        let mut expected_prev = [0u8; 32];
        let mut computed_head = [0u8; 32];
        let mut has_entries = false;
        for line in std::io::BufReader::new(file).lines() {
            let line = line.map_err(|e| anyhow::anyhow!("log read error: {e}"))?;
            let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) else {
                return Ok(LogIntegrity::Corrupt);
            };
            let Some(prev) = obj.get("prev_hash").and_then(|v| v.as_str()) else {
                return Ok(LogIntegrity::Corrupt);
            };
            let Ok(declared) = hex_decode(prev) else {
                return Ok(LogIntegrity::Corrupt);
            };
            if declared != expected_prev {
                return Ok(LogIntegrity::Corrupt);
            }
            computed_head = Sha256::digest(line.as_bytes()).into();
            expected_prev = computed_head;
            has_entries = true;
        }
        anyhow::ensure!(
            *self.chain_head.lock() == known_head,
            "provenance log changed during verification; retry"
        );
        if computed_head != known_head {
            return Ok(LogIntegrity::Corrupt);
        }
        let Some(signed_bytes) = signed_bytes else {
            return Ok(if has_entries {
                LogIntegrity::Unanchored
            } else {
                LogIntegrity::Empty
            });
        };
        match cose::verify_and_extract(&self.signing_key.verifying_key(), &signed_bytes) {
            Ok(stored_head) if stored_head.as_slice() == computed_head => {
                Ok(LogIntegrity::Verified)
            }
            _ => Ok(LogIntegrity::Corrupt),
        }
    }

    /// Replays the log into the live records and the chain head. A missing log is an empty
    /// one; an unreadable one is an error, since chaining new entries onto a guessed head
    /// would break the log.
    pub(super) fn load_log(
        path: Option<&Path>,
    ) -> Result<(
        crate::core::hash::FastMap<String, ProvenanceRecord>,
        [u8; 32],
        u64,
    )> {
        let mut next_sequence = 0;
        let mut map = crate::core::hash::FastMap::default();
        let mut head = [0u8; 32];
        let Some(path) = path else {
            return Ok((map, head, next_sequence));
        };
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok((map, head, next_sequence))
            }
            Err(e) => return Err(anyhow::anyhow!("failed to open provenance log: {e}")),
        };
        for line in std::io::BufReader::new(file).lines() {
            let line = match line {
                Ok(line) => line,
                // A crash can tear the last line mid-character; replay what precedes it.
                Err(e) if e.kind() == std::io::ErrorKind::InvalidData => continue,
                Err(e) => return Err(anyhow::anyhow!("log read error: {e}")),
            };
            head = Sha256::digest(line.as_bytes()).into();
            if let Ok(entry) = serde_json::from_str::<LogEntry>(&line) {
                let after = entry
                    .record
                    .sequence
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("provenance sequence is exhausted"))?;
                next_sequence = next_sequence.max(after);
                map.insert(entry.fact_id, entry.record);
            } else if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&line) {
                if obj.get("action").and_then(|a| a.as_str()) == Some("delete") {
                    if let Some(fid) = obj.get("fact_id").and_then(|f| f.as_str()) {
                        map.remove(fid);
                    }
                }
            }
        }
        Ok((map, head, next_sequence))
    }

    /// A missing status list means nothing is revoked. An unreadable one is an error rather
    /// than an empty list, which would report every revoked credential as valid.
    pub(super) fn load_revocation_bitstring(log_path: Option<&Path>) -> Result<Vec<u8>> {
        let Some(path) = log_path else {
            return Ok(Vec::new());
        };
        match std::fs::File::open(path.with_extension("statuslist")) {
            Ok(file) => {
                let mut bits = Vec::new();
                file.take((status_list::MAX_BYTES + 1) as u64)
                    .read_to_end(&mut bits)?;
                anyhow::ensure!(
                    bits.len() <= status_list::MAX_BYTES,
                    "revocation status list exceeds size limit"
                );
                Ok(bits)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(anyhow::anyhow!(
                "failed to read revocation status list: {e}"
            )),
        }
    }

    pub fn revoke_credential(&self, status_index: u64) -> Result<()> {
        // The index is caller-supplied: one past `usize` or past what can be allocated is an
        // error, not an overflow or an allocation abort.
        let out_of_range = || anyhow::anyhow!("status index {status_index} is out of range");
        let byte_idx = usize::try_from(status_index / 8).map_err(|_| out_of_range())?;
        let len = byte_idx.checked_add(1).ok_or_else(out_of_range)?;
        anyhow::ensure!(
            len <= status_list::MAX_BYTES,
            "status index exceeds list size limit"
        );
        let bit_idx = (status_index % 8) as u8;
        // The write guard covers the update and its persistence: concurrent revocations
        // reach the file in the order they change the list, so a later write never carries
        // an older list, and a failed write leaves the live list as the file has it.
        let mut bits = self.revocation_bitstring.write();
        let mut next = Vec::new();
        next.try_reserve(len.max(bits.len()))
            .map_err(|_| out_of_range())?;
        next.extend_from_slice(&bits);
        if next.len() < len {
            next.resize(len, 0);
        }
        next[byte_idx] |= 1 << bit_idx;
        if let Some(ref path) = self.log_path {
            // Replaced atomically: a torn list would read back as shorter, un-revoking the
            // credentials past the tear.
            super::super::durable_file::atomic_write(&path.with_extension("statuslist"), &next)
                .map_err(|e| anyhow::anyhow!("failed to persist revocation bitstring: {e}"))?;
        }
        *bits = next;
        Ok(())
    }

    pub fn is_revoked(&self, status_index: u64) -> bool {
        trust::status_bit(&self.revocation_bitstring.read(), status_index)
    }

    pub(super) fn published_status_list_len(&self, bits_len: usize) -> Option<usize> {
        let highest_issued = self
            .logical_clock
            .load(std::sync::atomic::Ordering::SeqCst)
            .saturating_sub(1);
        let issued_bytes = usize::try_from(highest_issued / 8).ok()?.checked_add(1)?;
        let length = status_list::MIN_BYTES.max(issued_bytes).max(bits_len);
        (length <= status_list::MAX_BYTES).then_some(length)
    }

    pub(super) fn local_status(&self, index: u64) -> StatusLookup {
        let bits = self.revocation_bitstring.read();
        let Some(length) = self.published_status_list_len(bits.len()) else {
            return StatusLookup::OutOfRange;
        };
        if index / 8 >= length as u64 {
            StatusLookup::OutOfRange
        } else {
            StatusLookup::Value(trust::status_bit(&bits, index))
        }
    }

    /// Create a full provenance record for a stored fact.
    pub fn create_fact_provenance(
        &self,
        fact_id: &str,
        content: &[u8],
        source_uri: Option<&str>,
    ) -> Result<ProvenanceRecord> {
        let content_hash: [u8; 32] = Sha256::digest(content).into();
        let mut record = ProvenanceRecord::now();
        record.source_uri = Some(source_uri.unwrap_or("").to_string());
        record.source_hash = Some(content_hash);
        record.issuer_did = Some(self.issuer_did.clone());
        record.sequence = self.reserve_sequences(1)?;

        let cose_envelope = cose::sign_fact_claim(
            &self.signing_key,
            fact_id,
            &content_hash,
            source_uri,
            record.timestamp_ms,
        )?;
        record.cose_envelope = Some(cose_envelope);

        let credential = vc::create_fact_credential(
            &self.issuer_did,
            fact_id,
            &content_hash,
            0, // dimensions filled by caller
            source_uri,
            None,
            record.sequence,
        )
        .with_status_list_url(&self.status_list_url);
        let signed_vc = vc::sign_credential(&self.signing_key, credential)?;
        record.vc_json = Some(serde_json::to_string(&signed_vc)?);

        #[cfg(feature = "provenance-scitt")]
        if let Some(ref endpoint) = self.scitt_endpoint {
            let claim = scitt::FactClaim {
                fact_id: fact_id.to_string(),
                content_hash: content_hash.to_vec(),
                timestamp_ms: record.timestamp_ms,
                source_uri: source_uri.map(String::from),
                store_id: None,
            };
            let statement = scitt::create_signed_statement(&self.signing_key, &claim)?;
            match scitt::submit_statement(endpoint, &statement) {
                Ok(receipt) => record.scitt_receipt = Some(receipt.receipt_bytes),
                Err(e) => tracing::warn!("SCITT submission failed: {e}"),
            }
        }

        self.store_record(fact_id, record.clone())?;
        Ok(record)
    }

    /// Create a full provenance record for a stored triple.
    pub fn create_triple_provenance(
        &self,
        params: &TripleProvenanceParams<'_>,
    ) -> Result<ProvenanceRecord> {
        let content_hash: [u8; 32] = Sha256::digest(params.content).into();
        let mut record = ProvenanceRecord::now();
        record.source_uri = params.source_uri.map(String::from);
        record.source_hash = Some(content_hash);
        record.issuer_did = Some(self.issuer_did.clone());
        record.sequence = self.reserve_sequences(1)?;

        let cose_envelope = cose::sign_fact_claim(
            &self.signing_key,
            params.fact_id,
            &content_hash,
            params.source_uri,
            record.timestamp_ms,
        )?;
        record.cose_envelope = Some(cose_envelope);

        let credential = vc::create_fact_credential(
            &self.issuer_did,
            params.fact_id,
            &content_hash,
            params.dimensions,
            params.source_uri,
            Some((params.subject, params.relation, params.object)),
            record.sequence,
        )
        .with_status_list_url(&self.status_list_url);
        let signed_vc = vc::sign_credential(&self.signing_key, credential)?;
        record.vc_json = Some(serde_json::to_string(&signed_vc)?);

        self.store_record(params.fact_id, record.clone())?;
        Ok(record)
    }

    pub fn create_batch_provenance(
        &self,
        items: &[(&str, &[u8], Option<&str>)],
    ) -> Result<Vec<ProvenanceRecord>> {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let leaf_hashes: Vec<[u8; 32]> = items
            .iter()
            .map(|(_, content, _)| Sha256::digest(content).into())
            .collect();
        let hashing = MerkleHashing::Rfc6962;
        let leaf_nodes: Vec<[u8; 32]> = leaf_hashes.iter().map(|h| hashing.leaf(h)).collect();
        let levels = Self::merkle_levels(&leaf_nodes, hashing);
        let merkle_root = levels[levels.len() - 1][0];
        let root_envelope = cose::sign_payload(
            &self.signing_key,
            &batch_root_claim(&merkle_root, leaf_nodes.len())?,
        )?;

        // One contiguous block of sequence numbers, then the per-record credential signatures
        // (most of a batch's cost) in parallel on the rayon pool, collected in item order.
        let first_sequence = self.reserve_sequences(items.len())?;
        let records = items
            .par_iter()
            .enumerate()
            .map(
                |(i, (fact_id, _, source_uri))| -> Result<ProvenanceRecord> {
                    let content_hash = leaf_hashes[i];
                    let mut record = ProvenanceRecord::now();
                    record.source_uri = Some(source_uri.unwrap_or("").to_string());
                    record.source_hash = Some(content_hash);
                    record.issuer_did = Some(self.issuer_did.clone());
                    record.sequence = first_sequence + i as u64;
                    record.cose_envelope = Some(root_envelope.clone());
                    record.merkle_proof = Some(Self::merkle_proof(&levels, i));

                    let credential = vc::create_fact_credential(
                        &self.issuer_did,
                        fact_id,
                        &content_hash,
                        0,
                        *source_uri,
                        None,
                        record.sequence,
                    )
                    .with_status_list_url(&self.status_list_url);
                    let signed_vc = vc::sign_credential(&self.signing_key, credential)?;
                    record.vc_json = Some(serde_json::to_string(&signed_vc)?);
                    Ok(record)
                },
            )
            .collect::<Result<Vec<_>>>()?;
        // One append, sync and head signature for the whole batch.
        let append = match self.log_path {
            Some(ref path) => Some(self.append_chained(
                path,
                records.len(),
                &self.signing_key,
                |i, prev_hash| {
                    serde_json::to_string(&LogEntryRef {
                        prev_hash,
                        fact_id: items[i].0,
                        record: &records[i],
                    })
                },
            )?),
            None => None,
        };
        let mut live = self.records.write();
        for ((fact_id, _, _), record) in items.iter().zip(&records) {
            live.insert(fact_id.to_string(), record.clone());
        }
        drop(live);
        if let Some(append) = append {
            append.finish()?;
        }
        Ok(records)
    }

    /// The Merkle tree's levels, leaves first and the root alone last.
    pub(super) fn merkle_levels(leaves: &[[u8; 32]], hashing: MerkleHashing) -> Vec<Vec<[u8; 32]>> {
        let mut levels = vec![if leaves.is_empty() {
            vec![[0u8; 32]]
        } else {
            leaves.to_vec()
        }];
        loop {
            let top = &levels[levels.len() - 1];
            if top.len() <= 1 {
                return levels;
            }
            let parents = Self::merkle_parents(top, hashing);
            levels.push(parents);
        }
    }

    /// One Merkle level up: the node hash of each adjacent pair, an odd last node carried
    /// unhashed.
    pub(super) fn merkle_parents(level: &[[u8; 32]], hashing: MerkleHashing) -> Vec<[u8; 32]> {
        level
            .chunks(2)
            .map(|pair| match pair {
                [left, right] => hashing.node(left, right),
                _ => pair[0],
            })
            .collect()
    }

    /// Leaf `index`'s inclusion proof, read from the levels of [`Self::merkle_levels`]: each
    /// level's sibling, flagged when it sits to the left; an odd last node has none.
    pub(super) fn merkle_proof(levels: &[Vec<[u8; 32]>], index: usize) -> Vec<(bool, [u8; 32])> {
        let mut idx = index;
        let mut proof = Vec::new();
        for level in &levels[..levels.len() - 1] {
            if let Some(sibling) = level.get(idx ^ 1) {
                proof.push((idx % 2 == 1, *sibling));
            }
            idx /= 2;
        }
        proof
    }

    /// Create a signed manifest for the entire store.
    pub fn create_store_manifest(
        &self,
        store_id: &str,
        store_data: &[u8],
        fact_count: usize,
        dimensions: u32,
        title: Option<&str>,
    ) -> Result<StoreManifest> {
        let store_hash: [u8; 32] = Sha256::digest(store_data).into();

        let params = c2pa_manifest::ManifestParams {
            store_id,
            fact_count,
            dimensions,
            store_hash: &store_hash,
            title,
            ingredients: Vec::new(),
        };

        let manifest = c2pa_manifest::create_manifest(&self.signing_key, &params)?;
        let cose_envelope = c2pa_manifest::sign_manifest(&self.signing_key, &manifest)?;

        let manifest_label = format!("urn:hms:manifest:{store_id}");
        let claim_json = serde_json::to_value(&manifest)?;
        let assertions: Vec<(&str, serde_json::Value)> = manifest
            .assertions
            .iter()
            .map(|a| (a.label.as_str(), a.data.clone()))
            .collect();
        let jumbf_box = jumbf::build_c2pa_manifest_box(
            &manifest_label,
            claim_json,
            cose_envelope.clone(),
            assertions,
        );
        let jumbf_bytes = jumbf_box.encode()?;

        Ok(StoreManifest {
            manifest_id: manifest_label,
            issuer_did: Some(self.issuer_did.clone()),
            created_ms: unix_millis(),
            fact_count,
            store_hash,
            cose_envelope: Some(cose_envelope),
            jumbf_manifest: Some(jumbf_bytes),
        })
    }

    pub(super) fn resolve_cose_verifying_key(
        record: &ProvenanceRecord,
        fallback: &ed25519_dalek::VerifyingKey,
    ) -> ed25519_dalek::VerifyingKey {
        if let Some(ref envelope) = record.cose_envelope {
            if let Ok(kid) = cose::extract_key_id(envelope) {
                if kid.len() == 32 {
                    if let Ok(key) = ed25519_dalek::VerifyingKey::from_bytes(
                        kid.as_slice().try_into().unwrap_or(&[0u8; 32]),
                    ) {
                        return key;
                    }
                }
            }
        }
        if let Some(ref issuer_did) = record.issuer_did {
            if let Ok(pk_bytes) = did::ed25519_from_did_key(issuer_did) {
                if let Ok(key) = ed25519_dalek::VerifyingKey::from_bytes(&pk_bytes) {
                    return key;
                }
            }
        }
        *fallback
    }

    /// Authenticity check: verifies the record's signatures **and** requires the
    /// key that produced them to be present in `trust`. A record whose embedded
    /// key is not trusted verifies as `valid == false` even if its signature is
    /// internally consistent. Use [`ProvenanceManager::self_trust`] to trust
    /// only this manager's own key.
    pub fn verify_fact_provenance(
        &self,
        record: &ProvenanceRecord,
        trust: &TrustStore,
    ) -> Result<VerificationResult> {
        let mut result = self.verify_signatures(record, Some(trust))?;
        let fallback = self.signing_key.verifying_key();
        let key = Self::resolve_cose_verifying_key(record, &fallback);
        result
            .details
            .push(trust_anchor_detail(trust.is_trusted(&key)));
        result.valid = result.details.iter().all(|d| d.passed) && !result.details.is_empty();
        Ok(result)
    }

    /// Integrity-only check: verifies the record's signatures against the key
    /// **embedded in the record itself**. This proves the record has not been
    /// altered since signing, but says nothing about *who* signed it — a record
    /// signed by any key passes. For authenticity use
    /// [`ProvenanceManager::verify_fact_provenance`] with a trust anchor.
    pub fn verify_fact_provenance_self_consistency(
        &self,
        record: &ProvenanceRecord,
    ) -> Result<VerificationResult> {
        self.verify_signatures(record, None)
    }

    /// The signature, revocation and binding checks of a record. A credential citing this
    /// manager's status list is revoked when the live list or `trust`'s copy says so; one
    /// citing another list is checked against `trust`'s copy of that list. Without one it
    /// falls back to the live list (credentials issued before a key rotation cite the list
    /// under the old DID), unless `trust` requires status lists.
    pub(super) fn verify_signatures(
        &self,
        record: &ProvenanceRecord,
        trust: Option<&TrustStore>,
    ) -> Result<VerificationResult> {
        let legacy_status_list = vc::status_list_credential_url(&self.issuer_did);
        let mut details = Vec::new();
        let fallback = self.signing_key.verifying_key();
        let verifying_key = Self::resolve_cose_verifying_key(record, &fallback);

        let mut payload = None;
        if let Some(ref envelope) = record.cose_envelope {
            let outcome = cose::verify_and_extract(&verifying_key, envelope);
            details.push(outcome_detail("cose_signature", &outcome));
            payload = outcome.ok();
        }

        let mut credential = None;
        if let Some(ref vc_json) = record.vc_json {
            details.push(outcome_detail(
                "vc_proof",
                &vc::verify_credential_json(vc_json),
            ));
            match serde_json::from_str::<vc::FactCredential>(vc_json) {
                Ok(parsed) => {
                    details.push(outcome_detail(
                        "vc_semantics",
                        &vc::validate_fact_credential(&parsed),
                    ));
                    details.push(revocation_detail(
                        &parsed,
                        trust.is_some_and(TrustStore::requires_status_lists),
                        |url, index| {
                            let trusted = trust
                                .map_or(StatusLookup::Missing, |t| t.status_lookup(url, index));
                            if url == self.status_list_url || url == legacy_status_list {
                                match (self.local_status(index), trusted) {
                                    (StatusLookup::Value(local), StatusLookup::Value(remote)) => {
                                        StatusLookup::Value(local || remote)
                                    }
                                    (StatusLookup::Value(local), _) => StatusLookup::Value(local),
                                    (StatusLookup::OutOfRange, StatusLookup::Value(remote)) => {
                                        StatusLookup::Value(remote)
                                    }
                                    (StatusLookup::OutOfRange, StatusLookup::Stale) => {
                                        StatusLookup::Stale
                                    }
                                    _ => StatusLookup::OutOfRange,
                                }
                            } else {
                                trusted
                            }
                        },
                    ));
                    credential = Some(parsed);
                }
                Err(e) => details.push(VerificationDetail {
                    check: "vc_parse".to_string(),
                    passed: false,
                    message: Some(e.to_string()),
                }),
            }
        }
        if !details.is_empty() {
            details.push(outcome_detail(
                "record_binding",
                &check_binding(
                    record,
                    payload.as_deref().map(|p| (p, &verifying_key)),
                    credential.as_ref(),
                ),
            ));
        }

        let all_passed = details.iter().all(|d| d.passed);

        Ok(VerificationResult {
            valid: all_passed && !details.is_empty(),
            issuer_did: record.issuer_did.clone(),
            timestamp_ms: record.timestamp_ms,
            details,
        })
    }

    pub fn signing_key(&self) -> &SigningKey {
        &self.signing_key
    }

    /// This manager's current verifying (public) key.
    pub fn verifying_key(&self) -> ed25519_dalek::VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// A trust store containing only this manager's own key. Pass it to the
    /// verify methods to authenticate records this manager signed.
    /// A trust store holding this manager's key and a snapshot of its revocation status list.
    pub fn self_trust(&self) -> TrustStore {
        let mut trust = TrustStore::trusting_key(&self.signing_key.verifying_key());
        let mut bits = self.revocation_bitstring.read().clone();
        if let Some(length) = self.published_status_list_len(bits.len()) {
            bits.resize(length, 0);
        }
        trust.trust_status_list(&self.status_list_url, bits.clone());
        let legacy_url = vc::status_list_credential_url(&self.issuer_did);
        if legacy_url != self.status_list_url {
            trust.trust_status_list(&legacy_url, bits);
        }
        trust
    }

    pub fn status_list_credential(&self) -> Result<status_list::StatusListCredential> {
        let highest_issued = self
            .logical_clock
            .load(std::sync::atomic::Ordering::SeqCst)
            .saturating_sub(1);
        let bits = self.revocation_bitstring.read().clone();
        status_list::create_status_list_credential(
            &self.signing_key,
            &self.status_list_url,
            &bits,
            highest_issued,
        )
    }

    pub fn create_sigstore_bundle(
        &self,
        content: &[u8],
        identity: Option<&str>,
    ) -> Result<sigstore::SigstoreBundle> {
        sigstore::create_local_bundle(&self.signing_key, content, identity)
    }

    /// Verify a Sigstore bundle against a caller-supplied trusted key. The key
    /// embedded in the bundle is ignored: authenticity comes from `trusted_key`,
    /// not from the bundle itself.
    pub fn verify_sigstore_bundle(
        &self,
        bundle: &sigstore::SigstoreBundle,
        content: &[u8],
        trusted_key: &ed25519_dalek::VerifyingKey,
    ) -> Result<()> {
        sigstore::verify_bundle_with_key(bundle, content, trusted_key)
    }

    /// Create a CAWG ICA identity assertion. `referenced` is `(url, alg, raw_hash)` and
    /// MUST include the hard binding. Returns the `cawg.identity` assertion JSON.
    pub fn create_cawg_assertion(
        &self,
        referenced: &[(String, String, Vec<u8>)],
        display_name: &str,
    ) -> Result<serde_json::Value> {
        cawg::create_identity_assertion_ica(&self.signing_key, referenced, display_name)
    }

    /// Verify a standalone `cawg.identity` ICA signature and issuer trust, returning
    /// the embedded VC. Complete identity assertion validation also requires the
    /// authenticated containing C2PA claim and its referenced assertions.
    pub fn verify_cawg_assertion(
        &self,
        assertion: &serde_json::Value,
        trust: &TrustStore,
    ) -> Result<serde_json::Value> {
        let embedded = cawg::ica_embedded_bytes(assertion)?;
        let vc = cawg::verify_cawg_ica(&embedded)?;
        let issuer = vc
            .get("issuer")
            .and_then(|v| {
                v.as_str()
                    .or_else(|| v.get("id").and_then(|id| id.as_str()))
            })
            .ok_or_else(|| anyhow::anyhow!("CAWG assertion VC has no issuer"))?;
        let key_bytes = did::ed25519_from_did_jwk(issuer)?;
        let key = ed25519_dalek::VerifyingKey::from_bytes(&key_bytes)
            .map_err(|e| anyhow::anyhow!("invalid CAWG issuer key: {e}"))?;
        if !trust.is_trusted(&key) {
            return Err(anyhow::anyhow!("CAWG issuer is not trusted: {issuer}"));
        }
        Ok(vc)
    }

    pub fn keri_event_log(&self) -> &RwLock<keri::KeyEventLog> {
        &self.kel
    }

    pub fn create_keri_interaction(
        &self,
        anchors: Vec<serde_json::Value>,
    ) -> Result<keri::KeyEvent> {
        self.kel
            .write()
            .interaction(&self.signing_key, anchors)
            .cloned()
    }

    pub fn verify_keri_log(&self) -> Result<()> {
        self.kel.read().verify()
    }

    /// Verify a store manifest's COSE envelope and optionally check the store hash.
    pub(super) fn resolve_manifest_verifying_key(
        manifest: &StoreManifest,
        fallback: &ed25519_dalek::VerifyingKey,
    ) -> ed25519_dalek::VerifyingKey {
        if let Some(ref envelope) = manifest.cose_envelope {
            if let Ok(kid) = cose::extract_key_id(envelope) {
                if let Ok(key) = ed25519_dalek::VerifyingKey::from_bytes(
                    kid.as_slice().try_into().unwrap_or(&[0u8; 32]),
                ) {
                    return key;
                }
            }
        }
        if let Some(ref issuer_did) = manifest.issuer_did {
            if let Ok(pk_bytes) = did::ed25519_from_did_key(issuer_did) {
                if let Ok(key) = ed25519_dalek::VerifyingKey::from_bytes(&pk_bytes) {
                    return key;
                }
            }
        }
        *fallback
    }

    /// Authenticity check for a store manifest: verifies its COSE signature and
    /// optional store hash, and requires the signing key to be present in
    /// `trust`. See [`ProvenanceManager::verify_fact_provenance`] for the
    /// self-consistency vs. authenticity distinction.
    pub fn verify_store_manifest(
        &self,
        manifest: &StoreManifest,
        store_data: Option<&[u8]>,
        trust: &TrustStore,
    ) -> Result<VerificationResult> {
        let mut result = self.verify_store_manifest_self_consistency(manifest, store_data)?;
        let fallback = self.signing_key.verifying_key();
        let key = Self::resolve_manifest_verifying_key(manifest, &fallback);
        result
            .details
            .push(trust_anchor_detail(trust.is_trusted(&key)));
        result.valid = result.details.iter().all(|d| d.passed) && !result.details.is_empty();
        Ok(result)
    }

    /// Integrity-only check for a store manifest: verifies signatures against
    /// the key embedded in the manifest. Does not establish authenticity.
    pub fn verify_store_manifest_self_consistency(
        &self,
        manifest: &StoreManifest,
        store_data: Option<&[u8]>,
    ) -> Result<VerificationResult> {
        let mut details = Vec::new();
        let fallback = self.signing_key.verifying_key();
        let verifying_key = Self::resolve_manifest_verifying_key(manifest, &fallback);

        if let Some(ref envelope) = manifest.cose_envelope {
            match cose::verify_and_extract(&verifying_key, envelope) {
                Ok(payload) => {
                    details.push(VerificationDetail {
                        check: "manifest_cose_signature".to_string(),
                        passed: true,
                        message: None,
                    });

                    // The signature only proves the key signed *some* payload: it must be a
                    // store manifest for this manifest's store hash, or an envelope signed for
                    // a fact, a head or another store verifies when pasted in.
                    let parsed = serde_json::from_slice::<c2pa_manifest::WarmManifest>(&payload)
                        .map_err(|e| {
                            anyhow::anyhow!("signed payload is not a store manifest: {e}")
                        });
                    let binding = parsed
                        .as_ref()
                        .map_err(|e| anyhow::anyhow!("{e}"))
                        .and_then(|warm_manifest| {
                            let signed = c2pa_manifest::signed_store_hash(warm_manifest)?;
                            if signed == hex_encode(&manifest.store_hash) {
                                Ok(())
                            } else {
                                Err(anyhow::anyhow!(
                                    "signed store hash differs from the manifest's"
                                ))
                            }
                        });
                    details.push(outcome_detail("manifest_binding", &binding));
                    if let Ok(ref warm_manifest) = parsed {
                        if let Some(data) = store_data {
                            let hash_ok = c2pa_manifest::validate_store_hash(warm_manifest, data)
                                .unwrap_or(false);
                            details.push(VerificationDetail {
                                check: "store_hash".to_string(),
                                passed: hash_ok,
                                message: if hash_ok {
                                    None
                                } else {
                                    Some("store hash mismatch".to_string())
                                },
                            });
                        }
                    }
                }
                Err(e) => details.push(VerificationDetail {
                    check: "manifest_cose_signature".to_string(),
                    passed: false,
                    message: Some(e.to_string()),
                }),
            }
        }

        let all_passed = details.iter().all(|d| d.passed);

        Ok(VerificationResult {
            valid: all_passed && !details.is_empty(),
            issuer_did: manifest.issuer_did.clone(),
            timestamp_ms: manifest.created_ms,
            details,
        })
    }
}
