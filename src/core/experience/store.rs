// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! The durable experience store: one [`Experience`] with a write-ahead journal and a snapshot
//! in a store directory (docs/file-format/EXPERIENCE.md).
//!
//! Every accepted change (an observation step, an episode description, an episode purge) is one
//! journal record, synced before the call that made it returns. A checkpoint writes the snapshot
//! for the journal's last sequence number, replaces the journal with an empty one and scrubs
//! what it replaced. A crash anywhere leaves the previous snapshot with the journal that
//! extends it, or the new snapshot with a journal whose records it already covers; opening
//! skips records at or below the snapshot's sequence number, so replay is idempotent.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{
    Experience, ExperienceConfig, ExperienceError, Ingested, OwnedObservation, PurgeReport, Result,
    SNAPSHOT_FILE,
};
use crate::core::durable_file::{atomic_write, scrub_remove_file, sync_directory};
use crate::core::failpoint;

/// The journal file next to the snapshot.
pub const JOURNAL_FILE: &str = "experience.wexj";
const NEXT_SUFFIX: &str = ".next";
const MAGIC: &[u8; 4] = b"WEXJ";
const VERSION: u8 = 1;
const HEADER: usize = 8;
/// A journaled tree nests a map and an array per level, so the decoder's default recursion limit
/// of 256 would refuse a tree the store accepted at a depth near 128. This covers the hard
/// depth cap plus the record wrapper.
const RECORD_RECURSION: usize = 2 * super::HARD_MAX_DEPTH + 32;

const FRAME_HEADER: usize = 8;
/// A frame holds one observation; the tree limits bound it far below this.
const MAX_FRAME_BYTES: usize = 256 << 20;
/// The journal length at which a write also checkpoints.
const CHECKPOINT_JOURNAL_BYTES: u64 = 64 << 20;

/// Seals and opens the bytes of an encrypted store.
pub(crate) trait Sealer: Send + Sync {
    fn seal(&self, plaintext: &[u8]) -> anyhow::Result<Vec<u8>>;
    fn unseal(&self, sealed: &[u8]) -> anyhow::Result<Vec<u8>>;
}

/// One journaled change.
#[derive(Clone, Debug, Serialize, Deserialize)]
enum Op {
    Ingest(OwnedObservation),
    Describe {
        episode: String,
        goal: String,
        outcome: String,
    },
    Purge {
        episode: String,
    },
}

#[derive(Serialize, Deserialize)]
struct Record {
    seq: u64,
    op: Op,
}

fn io(e: impl std::fmt::Display) -> ExperienceError {
    ExperienceError::Io(e.to_string())
}

fn snapshot_error(e: impl std::fmt::Display) -> ExperienceError {
    ExperienceError::Snapshot(e.to_string())
}

pub struct ExperienceStore {
    dir: PathBuf,
    sealer: Option<Arc<dyn Sealer>>,
    x: Experience,
    /// Sequence number of the last journaled change.
    seq: u64,
    /// Sequence number the snapshot covers.
    snapshot_seq: u64,
    /// Closed across a checkpoint, because the old journal is removed while it is replaced.
    journal: Option<File>,
    journal_bytes: u64,
    /// A checkpoint published its snapshot but did not finish replacing the journal, so the
    /// file on disk is scrubbed or covered by the snapshot and must not take another append.
    journal_stale: bool,
}

impl std::ops::Deref for ExperienceStore {
    type Target = Experience;
    fn deref(&self) -> &Experience {
        &self.x
    }
}

fn journal_header() -> [u8; HEADER] {
    let mut h = [0u8; HEADER];
    h[..4].copy_from_slice(MAGIC);
    h[4] = VERSION;
    h
}

/// Whether the directory holds experience data.
pub(crate) fn exists(dir: &Path) -> bool {
    [
        SNAPSHOT_FILE.to_string(),
        JOURNAL_FILE.to_string(),
        format!("{SNAPSHOT_FILE}{NEXT_SUFFIX}"),
    ]
    .iter()
    .any(|f| dir.join(f).exists())
}

impl ExperienceStore {
    /// Opens the store in `dir`, recovering from any interrupted write or checkpoint. A damaged
    /// snapshot or a damaged journal record that is not at the tail is an error.
    pub(crate) fn open(
        dir: &Path,
        cfg: ExperienceConfig,
        sealer: Option<Arc<dyn Sealer>>,
    ) -> Result<Self> {
        let main = dir.join(SNAPSHOT_FILE);
        let next = dir.join(format!("{SNAPSHOT_FILE}{NEXT_SUFFIX}"));
        // A checkpoint stopped before it published its snapshot. The staged file is written
        // atomically, so it is whole and at least as new as the published one, which may by now
        // be partly or fully overwritten by the scrub.
        if next.exists() {
            if main.exists() {
                scrub_remove_file(&main).map_err(io)?;
            }
            std::fs::rename(&next, &main).map_err(io)?;
            sync_directory(dir).map_err(io)?;
        }
        let (mut x, snapshot_seq) = match std::fs::read(&main) {
            Ok(bytes) => {
                let bytes = match &sealer {
                    Some(s) => s.unseal(&bytes).map_err(snapshot_error)?,
                    None => bytes,
                };
                Experience::from_bytes_at(&bytes, cfg)?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Experience::new(cfg), 0),
            Err(e) => return Err(io(e)),
        };

        let journal_path = dir.join(JOURNAL_FILE);
        let mut seq = snapshot_seq;
        let (mut valid_len, mut stale, mut purged) = (HEADER as u64, false, false);
        let mut journal_present = false;
        match std::fs::read(&journal_path) {
            // Zeros: a checkpoint scrubbed the old journal and stopped before unlinking it. Its
            // records are all in the snapshot that checkpoint published.
            Ok(bytes) if bytes.len() >= HEADER && bytes[..HEADER].iter().all(|&b| b == 0) => {
                if acknowledged_after_scrub(&bytes, snapshot_seq, sealer.as_deref()) {
                    return Err(snapshot_error(
                        "journal holds records after a scrubbed region; they are not in the snapshot",
                    ));
                }
            }
            Ok(bytes) if bytes.len() >= HEADER => {
                if &bytes[..4] != MAGIC || bytes[4] != VERSION {
                    return Err(snapshot_error("not an experience journal"));
                }
                journal_present = true;
                let mut at = HEADER;
                while at < bytes.len() {
                    let rest = &bytes[at..];
                    if rest.len() < FRAME_HEADER {
                        break;
                    }
                    let len = u32::from_le_bytes(rest[..4].try_into().expect("4 bytes")) as usize;
                    let crc = u32::from_le_bytes(rest[4..8].try_into().expect("4 bytes"));
                    if len > MAX_FRAME_BYTES || FRAME_HEADER + len > rest.len() {
                        // Runs past the end: an append the crash cut short.
                        break;
                    }
                    let body = &rest[FRAME_HEADER..FRAME_HEADER + len];
                    if crc32fast::hash(body) != crc {
                        if FRAME_HEADER + len == rest.len() {
                            break;
                        }
                        return Err(snapshot_error("damaged journal record"));
                    }
                    let plain = match &sealer {
                        Some(s) => s.unseal(body).map_err(snapshot_error)?,
                        None => body.to_vec(),
                    };
                    let record: Record = ciborium::de::from_reader_with_recursion_limit(
                        plain.as_slice(),
                        RECORD_RECURSION,
                    )
                    .map_err(|e| snapshot_error(format!("journal record: {e}")))?;
                    at += FRAME_HEADER + len;
                    valid_len = at as u64;
                    if record.seq <= snapshot_seq {
                        stale = true;
                        continue;
                    }
                    if record.seq != seq + 1 {
                        return Err(snapshot_error("journal sequence gap"));
                    }
                    seq = record.seq;
                    purged |= matches!(record.op, Op::Purge { .. });
                    Self::apply(&mut x, &record.op).map_err(|e| {
                        snapshot_error(format!("journal record {seq} does not replay: {e}"))
                    })?;
                }
                if (valid_len as usize) < bytes.len() {
                    // Drop the cut-short tail; it was never acknowledged.
                    let f = open_rw(&journal_path)?;
                    f.set_len(valid_len).map_err(io)?;
                    f.sync_all().map_err(io)?;
                }
            }
            // Missing, or shorter than a header: a checkpoint that stopped between removing
            // the old journal and creating the new one. Every record it held is in the snapshot.
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(e)),
        }

        let mut store = if journal_present {
            Self {
                dir: dir.to_path_buf(),
                sealer,
                x,
                seq,
                snapshot_seq,
                journal: Some(open_rw(&journal_path)?),
                journal_bytes: valid_len,
                journal_stale: false,
            }
        } else {
            Self::create_journal(dir, x, seq, snapshot_seq, sealer)?
        };
        // A purge the crash cut short, or records a finished checkpoint already covers: finish
        // the scrub so no forgotten text outlives the call that forgot it.
        if purged || stale {
            store.checkpoint()?;
        }
        Ok(store)
    }

    fn create_journal(
        dir: &Path,
        x: Experience,
        seq: u64,
        snapshot_seq: u64,
        sealer: Option<Arc<dyn Sealer>>,
    ) -> Result<Self> {
        let path = dir.join(JOURNAL_FILE);
        atomic_write(&path, &journal_header()).map_err(io)?;
        let journal = open_rw(&path)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            sealer,
            x,
            seq,
            snapshot_seq,
            journal: Some(journal),
            journal_bytes: HEADER as u64,
            journal_stale: false,
        })
    }

    /// An empty store in `dir`, creating the journal.
    pub(crate) fn create(
        dir: &Path,
        cfg: ExperienceConfig,
        sealer: Option<Arc<dyn Sealer>>,
    ) -> Result<Self> {
        Self::create_journal(dir, Experience::new(cfg), 0, 0, sealer)
    }

    fn apply(x: &mut Experience, op: &Op) -> Result<Ingested> {
        match op {
            Op::Ingest(o) => {
                let ingested = x.ingest(&o.as_observation())?;
                if o.goal.is_none() && o.outcome.is_none() {
                    return Ok(ingested);
                }
                let (goal, outcome) = x
                    .episode_description(&o.episode)
                    .map(|(g, o)| (g.to_string(), o.to_string()))
                    .unwrap_or_default();
                let (new_goal, new_outcome) = (
                    o.goal.clone().unwrap_or_else(|| goal.clone()),
                    o.outcome.clone().unwrap_or_else(|| outcome.clone()),
                );
                if (&new_goal, &new_outcome) == (&goal, &outcome) {
                    return Ok(ingested);
                }
                x.describe_episode(&o.episode, &new_goal, &new_outcome)?;
                Ok(Ingested::Applied)
            }
            Op::Describe {
                episode,
                goal,
                outcome,
            } => x
                .describe_episode(episode, goal, outcome)
                .map(|()| Ingested::Applied),
            Op::Purge { episode } => {
                x.purge_episode(episode);
                Ok(Ingested::Applied)
            }
        }
    }

    pub fn experience(&self) -> &Experience {
        &self.x
    }

    /// The journal sequence number of the last durable change.
    pub fn sequence(&self) -> u64 {
        self.seq
    }

    fn frame(&self, record: &Record) -> Result<Vec<u8>> {
        let mut plain = Vec::new();
        ciborium::into_writer(record, &mut plain).map_err(|e| io(e.to_string()))?;
        let body = match &self.sealer {
            Some(s) => s.seal(&plain).map_err(io)?,
            None => plain,
        };
        if body.len() > MAX_FRAME_BYTES {
            return Err(ExperienceError::Limit {
                what: "journal record bytes",
                limit: MAX_FRAME_BYTES,
                got: body.len(),
            });
        }
        let mut out = Vec::with_capacity(FRAME_HEADER + body.len());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&crc32fast::hash(&body).to_le_bytes());
        out.extend_from_slice(&body);
        Ok(out)
    }

    fn journal_file(&mut self) -> std::io::Result<&mut File> {
        if self.journal.is_none() {
            let f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(self.dir.join(JOURNAL_FILE))?;
            self.journal = Some(f);
        }
        Ok(self.journal.as_mut().expect("just opened"))
    }

    /// Appends the frames and syncs them. On failure the journal is cut back to its length
    /// before the call.
    fn append(&mut self, frames: &[u8]) -> Result<()> {
        if self.journal_stale {
            self.reset_journal()?;
        }
        let before = self.journal_bytes;
        let result = (|| -> std::io::Result<()> {
            use std::io::{Seek, SeekFrom};
            let journal = self.journal_file()?;
            journal.seek(SeekFrom::Start(before))?;
            failpoint::hit("experience-journal-append")?;
            journal.write_all(frames)?;
            failpoint::hit("experience-journal-written")?;
            journal.sync_data()?;
            failpoint::hit("experience-journal-synced")?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.journal_bytes = before + frames.len() as u64;
                Ok(())
            }
            Err(e) => {
                if let Some(f) = self.journal.as_ref() {
                    let _ = f.set_len(before);
                }
                Err(io(e))
            }
        }
    }

    /// Applies the operations in order and makes them durable with one sync. A failing
    /// operation stops the call: the operations before it stay applied and durable, and the
    /// error carries the position of the failing one.
    fn run(
        &mut self,
        ops: Vec<Op>,
        budget: Option<u64>,
    ) -> std::result::Result<Vec<Ingested>, (usize, ExperienceError)> {
        let mut outcomes = Vec::with_capacity(ops.len());
        let mut frames = Vec::new();
        let (start_seq, mut failed, mut ahead) = (self.seq, None, false);
        for (i, op) in ops.into_iter().enumerate() {
            // A step that adds a unit and finds none left is refused before it touches the
            // state, so the common rejection needs no reload.
            if let (Op::Ingest(o), Some(b)) = (&op, budget) {
                if self.x.quota_units() >= b && !self.x.has_step(&o.episode, o.step) {
                    failed = Some((i, ExperienceError::Quota(b)));
                    break;
                }
            }
            match Self::apply(&mut self.x, &op) {
                Ok(Ingested::Duplicate) => outcomes.push(Ingested::Duplicate),
                Ok(Ingested::Applied) => {
                    if let Some(b) = budget.filter(|b| self.x.quota_units() > *b) {
                        // Regions this step created took it over: applied in memory only.
                        ahead = true;
                        failed = Some((i, ExperienceError::Quota(b)));
                        break;
                    }
                    if let Err(e) = self.x.check_snapshot_size() {
                        // The snapshot could not be read back with this step in it.
                        ahead = true;
                        failed = Some((i, e));
                        break;
                    }
                    self.seq += 1;
                    let record = Record { seq: self.seq, op };
                    match self.frame(&record) {
                        Ok(f) => frames.extend_from_slice(&f),
                        Err(e) => {
                            // Applied in memory but cannot be journaled: resynchronize below.
                            self.seq -= 1;
                            ahead = true;
                            failed = Some((i, e));
                            break;
                        }
                    }
                    outcomes.push(Ingested::Applied);
                }
                Err(e) => {
                    failed = Some((i, e));
                    break;
                }
            }
        }
        let journaled = if frames.is_empty() {
            Ok(())
        } else {
            self.append(&frames)
        };
        if let Err(e) = journaled {
            self.seq = start_seq;
            return Err((0, self.resync(e)));
        }
        if ahead {
            if let Err(e) = self.reload() {
                return Err((0, e));
            }
        }
        if let Some(f) = failed {
            return Err(f);
        }
        if self.journal_bytes >= CHECKPOINT_JOURNAL_BYTES {
            self.checkpoint().map_err(|e| (0, e))?;
        }
        Ok(outcomes)
    }

    /// Rebuilds the in-memory state from the files, after a failure left it ahead of them.
    fn reload(&mut self) -> Result<()> {
        let fresh = Self::open(&self.dir, self.x.cfg.clone(), self.sealer.clone())?;
        *self = fresh;
        Ok(())
    }

    fn resync(&mut self, error: ExperienceError) -> ExperienceError {
        match self.reload() {
            Ok(()) => error,
            Err(reload) => ExperienceError::Io(format!("{error}; reload failed: {reload}")),
        }
    }

    /// Ingests observations in order. Returns one outcome per observation, or the position of
    /// the first failing one with its error; earlier ones stay stored.
    pub fn ingest_many(
        &mut self,
        observations: Vec<OwnedObservation>,
    ) -> std::result::Result<Vec<Ingested>, (usize, ExperienceError)> {
        self.ingest_many_within(observations, None)
    }

    /// [`Self::ingest_many`] that refuses a step which would take [`Experience::quota_units`]
    /// past `budget`; steps before it stay stored and nothing of it remains.
    pub fn ingest_many_within(
        &mut self,
        observations: Vec<OwnedObservation>,
        budget: Option<u64>,
    ) -> std::result::Result<Vec<Ingested>, (usize, ExperienceError)> {
        self.run(observations.into_iter().map(Op::Ingest).collect(), budget)
    }

    pub fn describe_episode(&mut self, episode: &str, goal: &str, outcome: &str) -> Result<()> {
        self.run(
            vec![Op::Describe {
                episode: episode.into(),
                goal: goal.into(),
                outcome: outcome.into(),
            }],
            None,
        )
        .map(|_| ())
        .map_err(|(_, e)| e)
    }

    /// Removes an episode durably: the purge is journaled, then a checkpoint writes the snapshot
    /// without it and scrubs the old snapshot and journal. `None` for an unknown episode.
    pub fn purge_episode(&mut self, episode: &str) -> Result<Option<PurgeReport>> {
        if self.x.episode_slot(episode).is_none() {
            return Ok(None);
        }
        let mut frames = Vec::new();
        let record = Record {
            seq: self.seq + 1,
            op: Op::Purge {
                episode: episode.into(),
            },
        };
        frames.extend_from_slice(&self.frame(&record)?);
        // Journal first: once it is durable the purge is acknowledged work that a crash
        // finishes on open.
        self.append(&frames)?;
        self.seq = record.seq;
        let report = self.x.purge_episode(episode);
        self.checkpoint()?;
        Ok(report)
    }

    /// Writes the snapshot for the current sequence, replaces the journal with an empty one and
    /// scrubs the files it replaced.
    pub fn checkpoint(&mut self) -> Result<()> {
        let main = self.dir.join(SNAPSHOT_FILE);
        let next = self.dir.join(format!("{SNAPSHOT_FILE}{NEXT_SUFFIX}"));
        let mut bytes = self.x.to_bytes_at(self.seq)?;
        if let Some(s) = &self.sealer {
            bytes = s.seal(&bytes).map_err(io)?;
        }
        atomic_write(&next, &bytes).map_err(io)?;
        failpoint::hit("experience-checkpoint-staged").map_err(io)?;
        if main.exists() {
            scrub_remove_file(&main).map_err(io)?;
        }
        failpoint::hit("experience-checkpoint-old-removed").map_err(io)?;
        std::fs::rename(&next, &main).map_err(io)?;
        sync_directory(&self.dir).map_err(io)?;
        failpoint::hit("experience-checkpoint-published").map_err(io)?;
        self.snapshot_seq = self.seq;

        self.journal_stale = true;
        self.reset_journal()
    }

    /// Replaces the journal with an empty one, scrubbing the file it replaces. Until this
    /// succeeds `journal_stale` stays set and no append is accepted.
    fn reset_journal(&mut self) -> Result<()> {
        let journal = self.dir.join(JOURNAL_FILE);
        self.journal = None;
        if journal.exists() {
            scrub_remove_file(&journal).map_err(io)?;
        }
        failpoint::hit("experience-journal-removed").map_err(io)?;
        atomic_write(&journal, &journal_header()).map_err(io)?;
        self.journal = Some(open_rw(&journal)?);
        self.journal_bytes = HEADER as u64;
        self.journal_stale = false;
        Ok(())
    }

    /// Whether a checkpoint would write anything new.
    pub fn is_dirty(&self) -> bool {
        self.seq != self.snapshot_seq
    }
}

/// Whether a journal that starts with a scrubbed header carries a valid record the snapshot does
/// not cover. A scrub zeroes the file from the front, so a crash during it leaves zeros followed
/// by old records, all of them in the snapshot; a record beyond the snapshot after zeros is an
/// acknowledged append that open must not drop.
fn acknowledged_after_scrub(bytes: &[u8], snapshot_seq: u64, sealer: Option<&dyn Sealer>) -> bool {
    let Some(first) = bytes[HEADER..].iter().position(|&b| b != 0).map(|i| i + HEADER) else {
        return false;
    };
    // The little-endian length may itself start with zero bytes.
    (first.saturating_sub(3).max(HEADER)..=first).any(|at| {
        let rest = &bytes[at..];
        if rest.len() < FRAME_HEADER {
            return false;
        }
        let len = u32::from_le_bytes(rest[..4].try_into().expect("4 bytes")) as usize;
        let crc = u32::from_le_bytes(rest[4..8].try_into().expect("4 bytes"));
        if len > MAX_FRAME_BYTES || FRAME_HEADER + len > rest.len() {
            return false;
        }
        let body = &rest[FRAME_HEADER..FRAME_HEADER + len];
        if crc32fast::hash(body) != crc {
            return false;
        }
        let plain = match sealer {
            Some(s) => match s.unseal(body) {
                Ok(p) => p,
                Err(_) => return false,
            },
            None => body.to_vec(),
        };
        ciborium::de::from_reader_with_recursion_limit::<Record, _>(
            plain.as_slice(),
            RECORD_RECURSION,
        )
        .is_ok_and(|r| r.seq > snapshot_seq)
    })
}

fn open_rw(path: &Path) -> Result<File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(io)
}
