// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! The durable record of purges whose deleted text has not been scrubbed yet.
//!
//! A purge is: [`WarmCore::mark_purge_pending`] (durable), the deletes, then a scrubbing
//! compaction. The marker is a ledger, not a flag: each purge owns a ticket, and a scrubbing
//! compaction removes only the tickets whose deletes had committed before it took its snapshot.
//! One purge finishing therefore never erases the evidence of another that is still in flight.
//! An open that finds any ticket completes the purge before handing the store out, or, when
//! it cannot (a full disk), opens with the purge still pending and says so.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::core::sync::Mutex;

/// Name of the ledger file in the store directory. It exists exactly while a purge is pending.
pub(super) const PURGE_PENDING: &str = "purge.pending";

/// Committed tickets that make a deferred purge scrub at once instead of waiting for the next
/// compaction or open.
pub(super) const DEFERRED_BATCH: usize = 64;

const HEADER: &str = "purge pending\n";

struct Entries {
    next: u64,
    /// Ticket to whether its deletes have all committed.
    entries: BTreeMap<u64, bool>,
    /// Why the last attempt to finish pending purges failed.
    last_error: Option<String>,
}

pub(super) struct PurgeLedger {
    path: PathBuf,
    state: Mutex<Entries>,
}

impl PurgeLedger {
    /// Reads the ledger of a store directory. Whatever it holds belonged to a process that is
    /// gone, so every ticket counts as committed; a file without a readable ticket (the
    /// pre-ticket marker) is one ticket.
    pub(super) fn open(dir: &Path) -> Result<Self> {
        let path = dir.join(PURGE_PENDING);
        let mut entries = BTreeMap::new();
        let mut next = 0;
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                for id in text
                    .lines()
                    .filter_map(|line| line.strip_prefix("ticket ")?.trim().parse::<u64>().ok())
                {
                    entries.insert(id, true);
                    next = next.max(id + 1);
                }
                if entries.is_empty() {
                    entries.insert(next, true);
                    next += 1;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
        }
        Ok(Self {
            path,
            state: Mutex::new(Entries {
                next,
                entries,
                last_error: None,
            }),
        })
    }

    fn persist(&self, entries: &BTreeMap<u64, bool>) -> Result<()> {
        if entries.is_empty() {
            match std::fs::remove_file(&self.path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.into()),
            }
            return crate::core::durable_file::sync_directory(
                self.path.parent().unwrap_or_else(|| Path::new(".")),
            );
        }
        let mut text = String::from(HEADER);
        for id in entries.keys() {
            text.push_str(&format!("ticket {id}\n"));
        }
        crate::core::durable_file::atomic_write(&self.path, text.as_bytes())
    }

    /// Adds a ticket and makes it durable before returning.
    pub(super) fn mark(&self) -> Result<u64> {
        let mut state = self.state.lock();
        let id = state.next;
        state.entries.insert(id, false);
        if let Err(error) = self.persist(&state.entries) {
            state.entries.remove(&id);
            return Err(error);
        }
        state.next += 1;
        Ok(id)
    }

    /// Records that every delete of the ticket has committed.
    pub(super) fn commit(&self, id: u64) {
        if let Some(committed) = self.state.lock().entries.get_mut(&id) {
            *committed = true;
        }
    }

    pub(super) fn contains(&self, id: u64) -> bool {
        self.state.lock().entries.contains_key(&id)
    }

    /// Tickets whose deletes have committed: a compaction that snapshots after this call
    /// covers all of them.
    pub(super) fn committed(&self) -> Vec<u64> {
        let state = self.state.lock();
        state
            .entries
            .iter()
            .filter_map(|(id, committed)| committed.then_some(*id))
            .collect()
    }

    /// Removes the tickets a finished scrubbing compaction covered.
    pub(super) fn clear(&self, covered: &[u64]) -> Result<()> {
        let mut state = self.state.lock();
        let before = state.entries.clone();
        for id in covered {
            state.entries.remove(id);
        }
        if before.len() != state.entries.len() {
            if let Err(error) = self.persist(&state.entries) {
                state.entries = before;
                return Err(error);
            }
        }
        state.last_error = None;
        Ok(())
    }

    pub(super) fn record_failure(&self, error: &anyhow::Error) {
        self.state.lock().last_error = Some(format!("{error:#}"));
    }

    pub(super) fn status(&self) -> PurgeStatus {
        let state = self.state.lock();
        PurgeStatus {
            pending: state.entries.len(),
            committed: state.entries.values().filter(|c| **c).count(),
            last_error: state.last_error.clone(),
        }
    }
}

/// How many purges are waiting for a scrubbing compaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurgeStatus {
    /// Purges not yet scrubbed: the deleted text of each may still be in the journal.
    pub pending: usize,
    /// Of those, the purges whose deletes have all committed (the next scrubbing compaction
    /// covers them).
    pub committed: usize,
    /// Why the last attempt to finish them failed, if it did.
    pub last_error: Option<String>,
}

/// One purge in progress: created before its deletes, ended by
/// [`WarmCore::finish_purge`](super::WarmCore::finish_purge) or
/// [`WarmCore::defer_purge`](super::WarmCore::defer_purge). A ticket dropped otherwise (an
/// error between the mark and the end) counts as committed, which is safe because a scrub of
/// text that was never deleted removes nothing.
pub struct PurgeTicket {
    id: u64,
    ledger: Arc<PurgeLedger>,
}

impl PurgeTicket {
    pub(super) fn new(id: u64, ledger: Arc<PurgeLedger>) -> Self {
        Self { id, ledger }
    }

    pub(super) fn id(&self) -> u64 {
        self.id
    }

    pub(super) fn ledger(&self) -> &PurgeLedger {
        &self.ledger
    }
}

impl Drop for PurgeTicket {
    fn drop(&mut self) {
        self.ledger.commit(self.id);
    }
}
