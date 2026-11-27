// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
#[error("data mutation is durable, but {effect} failed; reopen or reconcile before retrying")]
pub struct CommittedMutationError {
    pub effect: &'static str,
    #[source]
    pub source: anyhow::Error,
}

#[derive(Debug, Error)]
#[error("data mutation outcome is indeterminate after {effect} failed; reopen or reconcile before retrying")]
pub struct IndeterminateMutationError {
    pub effect: &'static str,
    #[source]
    pub source: anyhow::Error,
}

/// Another engine holds the store's writer lock. `holder_pid` is the pid its owner recorded in
/// the lock file; it is `None` when the file could not be read (Windows denies reads of a
/// locked file) or the holder had not written it yet.
#[derive(Debug, Error)]
#[error(
    "WARM store {} is already open by another engine{}; concurrent writers are not supported \
     (open the store read-only to read it alongside the writer)",
    .path.display(),
    holder_note(.holder_pid)
)]
pub struct StoreLockedError {
    pub path: std::path::PathBuf,
    pub holder_pid: Option<u32>,
}

fn holder_note(pid: &Option<u32>) -> String {
    pid.map_or_else(|| " (holder unknown)".into(), |pid| format!(" (pid {pid})"))
}

/// A mutation was attempted on a store opened with `open_read_only`.
#[derive(Debug, Error)]
#[error("the store is open read-only; open it for writing to change it")]
pub struct ReadOnlyError;

pub(crate) fn after_commit<T>(
    effect: &'static str,
    result: anyhow::Result<T>,
) -> anyhow::Result<T> {
    result.map_err(|source| CommittedMutationError { effect, source }.into())
}

#[derive(Debug, Error, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "node-api",
    derive(ts_rs::TS),
    ts(export, export_to = "types/")
)]
#[cfg_attr(feature = "server", derive(schemars::JsonSchema))]
#[serde(tag = "type", content = "message")]
pub enum WarmError {
    #[error("Invalid parameter: {details}")]
    InvalidParam { details: String },

    #[error("Storage failure: {context}")]
    StorageFailure { context: String },

    #[error("Encoding failed: {context}")]
    EncodingFailure { context: String },

    #[error("Query failed: {context}")]
    QueryFailure { context: String },

    #[error("Index not trained: {index_type}")]
    IndexNotTrained { index_type: String },

    #[error("Capacity exceeded: {limit}")]
    CapacityExceeded { limit: String },

    #[error("Internal error (code {code}): {context}")]
    Internal { code: i32, context: String },
}
