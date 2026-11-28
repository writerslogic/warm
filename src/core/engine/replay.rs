// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl WarmCore {
    /// Bundle vectors respecting the PrivacyConfig.
    /// When dp_enabled, uses Laplace noise with the configured epsilon.
    pub fn bundle<V: std::borrow::Borrow<EntangledHVec>>(&self, vectors: &[V]) -> EntangledHVec {
        if self.config.privacy.dp_enabled {
            EntangledHVec::bundle_dp_in_space(vectors, self.config.privacy.epsilon, self.dimensions)
        } else {
            EntangledHVec::bundle(vectors)
        }
    }

    pub(super) fn maybe_encrypt<'a>(&self, data: &'a [u8]) -> Result<std::borrow::Cow<'a, [u8]>> {
        #[cfg(feature = "security")]
        if let Some(ref enc) = self.encryption {
            return enc.encrypt(data).map(std::borrow::Cow::Owned);
        }
        Ok(std::borrow::Cow::Borrowed(data))
    }

    /// Decrypts `data` when the store is encrypted; a plaintext buffer is returned as is.
    pub(super) fn maybe_decrypt(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        #[cfg(feature = "security")]
        if let Some(ref enc) = self.encryption {
            return enc.decrypt(&data);
        }
        Ok(data)
    }

    pub(super) fn load_from_log(&self) -> Result<()> {
        let shards = self.shards.write();
        self.replay_from(0, &shards)?;
        self.rebuild_meaning_indices();
        Ok(())
    }

    pub(super) fn rebuild_meaning_indices(&self) {
        if let Some(atoms) = &self.atom_memory {
            atoms.rebuild_indices();
        }
        if let Some(composites) = &self.composite_memory {
            composites.rebuild_indices();
        }
    }

    /// Applies every committed frame from journal offset `from` to the end of the journal, in
    /// order, and returns the offset after the last one.
    pub(super) fn replay_from(&self, from: usize, shards: &ShardSet) -> Result<usize> {
        let mut offset = from;
        while offset < self.arena.stats().used_bytes {
            let (data, _) = self
                .arena
                .read_frame(offset)
                .with_context(|| format!("cannot replay arena frame at {offset}"))?;
            // Only an encrypted store can fail here, and the first frame that does is almost
            // always a wrong passphrase rather than damage, so say so instead of hiding it.
            let payload = self.maybe_decrypt(data).with_context(|| {
                format!("cannot decrypt arena frame at {offset}: wrong passphrase, or the store is corrupted")
            })?;
            if let Some(json) = payload.strip_prefix(mutation::TRANSACTION_MAGIC) {
                self.replay_frame(json, shards)?;
            } else if let Some(body) = payload.strip_prefix(journal::BINARY_MAGIC) {
                self.replay_binary_frame(body, shards)
                    .with_context(|| format!("cannot replay arena frame at {offset}"))?;
            } else {
                anyhow::bail!(match super::super::format::scan::classify_payload(&payload) {
                    super::super::format::scan::PayloadKind::OtherTransactionVersion(version)
                        if version.parse::<u32>().is_ok_and(|v| v > 3) =>
                    {
                        format!("journal frame format HMS-TXN-{version} is newer than the supported HMS-TXN-3; open the store with a release that supports it")
                    }
                    _ => "unsupported transaction format; re-encode original sources into a new store".to_string(),
                });
            }
            let next = self.arena.next_offset(offset)?;
            // The frame is applied; its pages stay on disk for the next replay or compaction.
            self.arena
                .release_pages(offset, next.saturating_sub(offset));
            offset = next;
        }
        Ok(offset)
    }

    /// Decodes one committed journal frame and applies it in order, `REPLAY_CHUNK` mutations
    /// at a time, so a frame written by one large batch is never held decoded in full. The
    /// frame passed its checksum; an invalid mutation still fails the open, as before.
    /// Applies replayed mutations; ones a store served from a checkpoint cannot take
    /// rebuild it first, as they would have when committed.
    pub(super) fn replay_mutations(&self, mutations: &[Mutation], shards: &ShardSet) -> Result<()> {
        if !self.checkpoint_accepts(mutations) {
            self.hydrate_documents()?;
        }
        self.validate_audit_transitions(mutations)?;
        self.apply_mutations(&self.prepare_mutations(mutations), shards)?;
        let segment = self.documents.read().text_segment();
        segment.map_or(Ok(()), |segment| segment.drain_scrubs())
    }

    pub(super) fn replay_frame(&self, json: &[u8], shards: &ShardSet) -> Result<()> {
        struct Chunked<'a, F> {
            apply: F,
            failure: &'a mut Option<anyhow::Error>,
        }
        impl<'de, F: FnMut(&[Mutation]) -> Result<()>> serde::de::Visitor<'de> for Chunked<'_, F> {
            type Value = ();
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a sequence")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                mut self,
                mut seq: A,
            ) -> std::result::Result<(), A::Error> {
                let mut chunk = Vec::with_capacity(REPLAY_CHUNK);
                loop {
                    let next = seq.next_element::<Mutation>()?;
                    let done = next.is_none();
                    chunk.extend(next);
                    if chunk.len() == REPLAY_CHUNK || (done && !chunk.is_empty()) {
                        if let Err(error) = (self.apply)(&chunk) {
                            *self.failure = Some(error);
                            return Err(serde::de::Error::custom("journal replay failed"));
                        }
                        chunk.clear();
                    }
                    if done {
                        return Ok(());
                    }
                }
            }
        }
        let mut failure = None;
        let mut decoder = serde_json::Deserializer::from_slice(json);
        let decoded = serde::Deserializer::deserialize_seq(
            &mut decoder,
            Chunked {
                apply: |chunk: &[Mutation]| {
                    for mutation in chunk {
                        self.validate_mutation(mutation)?;
                    }
                    self.replay_mutations(chunk, shards)
                },
                failure: &mut failure,
            },
        );
        if let Some(error) = failure {
            return Err(error);
        }
        decoded?;
        decoder.end()?;
        Ok(())
    }

    /// [`Self::replay_frame`] for an `HMS-TXN-3` payload body: mutations are decoded and
    /// applied `REPLAY_CHUNK` at a time, the frame's term dictionary held meanwhile.
    pub(super) fn replay_binary_frame(&self, body: &[u8], shards: &ShardSet) -> Result<()> {
        let mut decoder = journal::BinaryDecoder::new(body)?;
        let mut chunk = Vec::with_capacity(REPLAY_CHUNK);
        loop {
            let next = decoder.next_mutation()?;
            let done = next.is_none();
            chunk.extend(next);
            if chunk.len() == REPLAY_CHUNK || (done && !chunk.is_empty()) {
                for mutation in &chunk {
                    self.validate_mutation(mutation)?;
                }
                self.replay_mutations(&chunk, shards)?;
                chunk.clear();
            }
            if done {
                return Ok(());
            }
        }
    }

    /// `mutations` as a journal payload in this store's format.
    pub(super) fn transaction_payload(&self, mutations: &[Mutation]) -> Result<Vec<u8>> {
        match self.journal {
            journal::JournalFormat::Json => mutation::transaction_payload(mutations),
            journal::JournalFormat::Binary => journal::binary_payload(mutations),
        }
    }
}
