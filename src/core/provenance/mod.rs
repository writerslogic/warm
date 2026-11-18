// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

pub mod access;
pub mod c2pa_manifest;
pub mod cawg;
pub mod cose;
pub mod did;
pub mod jumbf;
pub mod keri;
pub mod scitt;
pub mod sigstore;
pub mod status_list;
pub mod trust;
pub mod types;
pub mod vc;

use anyhow::{Context, Result};
use ed25519_dalek::SigningKey;
use parking_lot::{Mutex, RwLock};
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

use crate::core::hash::hex as hex_encode;
use trust::StatusLookup;
use trust::TrustStore;

use types::{
    DeletionRecord, ProvenanceRecord, StoreManifest, VerificationDetail, VerificationResult,
};

pub struct TripleProvenanceParams<'a> {
    pub fact_id: &'a str,
    pub content: &'a [u8],
    pub dimensions: u32,
    pub subject: &'a str,
    pub relation: &'a str,
    pub object: &'a str,
    pub source_uri: Option<&'a str>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct LogEntry {
    prev_hash: String,
    fact_id: String,
    record: ProvenanceRecord,
}

/// [`LogEntry`] borrowing its record, serialized to the same bytes.
#[derive(serde::Serialize)]
struct LogEntryRef<'a> {
    prev_hash: String,
    fact_id: &'a str,
    record: &'a ProvenanceRecord,
}

/// Only `Verified` authenticates the complete available log against its signed head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogIntegrity {
    Verified,
    /// No log or anchor exists. This does not authenticate an empty history.
    Empty,
    Unanchored,
    Corrupt,
    /// An in-memory manager has no persistent history to verify.
    Volatile,
}

struct ChainedAppend<'a> {
    _head: parking_lot::MutexGuard<'a, [u8; 32]>,
    anchor: Result<()>,
}

impl ChainedAppend<'_> {
    fn finish(self) -> Result<()> {
        super::error::after_commit("provenance signed head", self.anchor)
    }
}

pub struct ProvenanceManager {
    signing_key: SigningKey,
    issuer_did: String,
    status_list_url: String,
    records: RwLock<crate::core::hash::FastMap<String, ProvenanceRecord>>,
    chain_head: Mutex<[u8; 32]>,
    logical_clock: std::sync::atomic::AtomicU64,
    log_path: Option<PathBuf>,
    revocation_bitstring: RwLock<Vec<u8>>,
    kel: RwLock<keri::KeyEventLog>,
    #[cfg(feature = "provenance-scitt")]
    scitt_endpoint: Option<String>,
}

#[cfg(test)]
mod binding_tests;
mod manager;
mod merkle;
#[cfg(test)]
mod tests;
mod util;
mod verify;
use self::merkle::*;
use self::util::*;
pub use self::verify::*;
