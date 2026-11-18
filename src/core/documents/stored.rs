// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct StoredChunk {
    pub id: String,
    /// Inline text, or none when the store moved it to its text segment (`text_at`) or the
    /// document was stored without text.
    pub text: Option<String>,
    /// Process-local offset, stored length and plaintext CRC32; rebuilt from authoritative records.
    #[serde(skip)]
    pub text_at: Option<(u64, u32, u32)>,
    pub start_byte: u32,
    pub end_byte: u32,
    pub vector: EntangledHVec,
    pub embedding: Option<Embedding>,
    pub terms: StoredTerms,
    pub word_count: usize,
    /// Whether `terms` were stemmed at ingest; chunks stored before stemming existed match
    /// the unstemmed query terms.
    #[serde(default)]
    pub stemmed: bool,
    /// Whether `terms` came from a store [`Analyzer`] other than [`Analyzer::Legacy`] (binary
    /// chunk flag bit 7), so a release that predates analyzers refuses the chunk rather than
    /// matching it with the legacy analysis.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub analyzed: bool,
    /// Whether `vector` is the chunk's encoding, awaits deferred encoding, or is deliberately
    /// absent; `Pending` and `Lexical` chunks carry an empty vector and are not in the vector
    /// index. Absent from the serialized form when `Encoded`.
    #[serde(default, skip_serializing_if = "VectorState::is_encoded")]
    pub vector_state: VectorState,
}

/// A stored document's metadata object. Documents of one batch whose metadata is equal share
/// one allocation ([`SharedMetadata::intern`]): a one-entry JSON object is a 632-byte map node.
/// It serializes exactly as the value it holds.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SharedMetadata(pub(super) Arc<Value>);

impl SharedMetadata {
    /// Shares the allocation of an earlier equal value in `seen`, or records this one.
    pub(crate) fn intern(
        &mut self,
        seen: &mut std::collections::HashMap<u64, Vec<SharedMetadata>>,
    ) {
        use std::hash::Hasher;
        struct Feed<'a>(&'a mut std::collections::hash_map::DefaultHasher);
        impl std::io::Write for Feed<'_> {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.write(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        if serde_json::to_writer(Feed(&mut hasher), &*self.0).is_err() {
            return;
        }
        let candidates = seen.entry(hasher.finish()).or_default();
        match candidates.iter().find(|other| *other.0 == *self.0) {
            Some(other) => self.0 = Arc::clone(&other.0),
            None if candidates.len() < 4 => candidates.push(self.clone()),
            None => {}
        }
    }
}

impl From<Value> for SharedMetadata {
    fn from(value: Value) -> Self {
        Self(Arc::new(value))
    }
}

impl Deref for SharedMetadata {
    type Target = Value;

    fn deref(&self) -> &Value {
        &self.0
    }
}

impl std::ops::DerefMut for SharedMetadata {
    fn deref_mut(&mut self) -> &mut Value {
        Arc::make_mut(&mut self.0)
    }
}

impl Serialize for SharedMetadata {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct StoredDocument {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub vector_record: bool,
    #[serde(deserialize_with = "deserialize_compact_string")]
    pub id: String,
    #[serde(default, deserialize_with = "deserialize_compact_uri")]
    pub source_uri: Option<String>,
    #[serde(deserialize_with = "deserialize_compact_string")]
    pub version: String,
    #[serde(deserialize_with = "deserialize_compact_metadata")]
    pub metadata: SharedMetadata,
    #[serde(deserialize_with = "deserialize_compact_chunks")]
    pub chunks: Vec<StoredChunk>,
}

pub(super) fn deserialize_compact_chunks<'de, D>(
    deserializer: D,
) -> Result<Vec<StoredChunk>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let mut chunks = Vec::<StoredChunk>::deserialize(deserializer)?;
    compact_chunks(&mut chunks);
    Ok(chunks)
}

pub(super) fn deserialize_compact_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let mut value = String::deserialize(deserializer)?;
    compact_string(&mut value);
    Ok(value)
}

pub(super) fn deserialize_compact_uri<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let mut value = Option::<String>::deserialize(deserializer)?;
    if let Some(value) = &mut value {
        compact_string(value);
    }
    Ok(value)
}

pub(super) fn deserialize_compact_metadata<'de, D>(
    deserializer: D,
) -> Result<SharedMetadata, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let mut value = Value::deserialize(deserializer)?;
    compact_metadata(&mut value);
    Ok(value.into())
}

pub(super) fn compact_vec<T>(values: &mut Vec<T>) {
    if values.capacity() != values.len() {
        *values = std::mem::take(values).into_boxed_slice().into_vec();
    }
}

pub(super) fn compact_string(value: &mut String) {
    if value.capacity() != value.len() {
        *value = std::mem::take(value).into_boxed_str().into_string();
    }
}

pub(super) fn compact_metadata(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values.iter_mut() {
                compact_metadata(value);
            }
            compact_vec(values);
        }
        Value::Object(values) => {
            if values.keys().any(|key| key.capacity() != key.len()) {
                *values = std::mem::take(values)
                    .into_iter()
                    .map(|(mut key, value)| {
                        compact_string(&mut key);
                        (key, value)
                    })
                    .collect();
            }
            for value in values.values_mut() {
                compact_metadata(value);
            }
        }
        Value::String(value) => compact_string(value),
        _ => {}
    }
}

pub(super) fn compact_chunks(chunks: &mut Vec<StoredChunk>) {
    for chunk in chunks.iter_mut() {
        compact_string(&mut chunk.id);
        if let Some(text) = &mut chunk.text {
            compact_string(text);
        }
        compact_vec(&mut chunk.vector.indices);
    }
    compact_vec(chunks);
}
