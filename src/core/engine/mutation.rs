// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use anyhow::{ensure, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

use super::{ShardSet, WarmCore};
use crate::core::{
    documents::StoredDocument,
    entangled::EntangledHVec,
    rules::CompositionRule,
    types::{Relation, RelationType},
};

pub(super) const TRANSACTION_MAGIC: &[u8] = crate::core::format::TRANSACTION_MAGIC;
pub(super) const MEANING_PREFIX: &str = "hms:meaning:";
/// Batches of at least this many mutations are validated on the thread pool.
const PARALLEL_VALIDATION: usize = 64;
/// Batches of at least this many mutations are serialized on the thread pool.
const PARALLEL_SERIALIZATION: usize = 16;

impl WarmCore {}

mod apply;
mod batch;
mod commit;
#[cfg(test)]
mod tests;
mod validate;
pub(crate) use self::batch::*;
