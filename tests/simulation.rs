// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Deterministic simulation of the document store, the memory API, compaction and historical
//! views against a naive bitemporal reference model.
//!
//! Each seed generates one operation sequence: plain documents written, updated and deleted;
//! memory facts with validity intervals, event times and explicit supersession, retried,
//! conflicting, forgotten and raw-deleted; a manual clock that stands still, advances and steps
//! back; foreground and background compactions (cancelled, busy, with writes in the tail);
//! flushes, reopens, and opens of a copy taken while the engine runs (the files a killed process
//! leaves). After every step the live store must agree with the model on the listing,
//! counts, search result sets and every fact's status `as_of`/`known_at` a bank of boundary
//! times. At every epoch boundary (before a compaction or reopen, and at the end) each recorded
//! journal position is replayed as a historical view and must equal the live state recorded
//! there. A multiset hash of the live records must survive compaction and reopen, and after a
//! published compaction no file may hold the text of a dead document.
//!
//! `WARM_SIM_SEED=<n>` replays one seed; `WARM_SIM_SEEDS=<n>` sets the count for the ignored
//! long run. Seeds reproduce the operation sequence; the interleaving of a background
//! compaction with its tail writes depends on the scheduler.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};
use writerslogic_warm::core::memory_api::*;
use writerslogic_warm::core::WarmConfig;
use writerslogic_warm::{
    CompactionOptions, CompactionStatus, DocumentInput, HistoryError, JournalPosition,
    SearchOptions, WarmCore,
};

const DIM: u32 = 1024;
const K: u32 = 1000;
const OPS: usize = 26;
const FACT_IDS: usize = 8;
const DOC_IDS: usize = 5;
const WORDS: usize = 5;
const T0: Millis = 1_000;

// ---------------------------------------------------------------------------------------------
// Utilities

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        (!items.is_empty()).then(|| &items[self.below(items.len())])
    }
}

fn word(i: usize) -> String {
    format!("w{i}q")
}

/// Order-independent control total: the wrapping sum of a 128-bit hash per record.
fn record_hash(parts: &[&str]) -> u128 {
    let half = |salt: u8| {
        let mut h = DefaultHasher::new();
        salt.hash(&mut h);
        parts.hash(&mut h);
        h.finish() as u128
    };
    (half(1) << 64) | half(2)
}

fn history_error(error: anyhow::Error) -> HistoryError {
    error
        .downcast::<HistoryError>()
        .unwrap_or_else(|e| panic!("not a history error: {e:#}"))
}

fn files_containing(dir: &Path, needle: &[u8], out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
        let path = entry.path();
        // The open store holds an exclusive lock on its lock file, which Windows refuses to
        // read; the file never holds store content.
        if path.file_name().is_some_and(|name| name == ".hms.lock") {
            continue;
        }
        if path.is_dir() {
            files_containing(&path, needle, out);
        } else if std::fs::read(&path)
            .unwrap()
            .windows(needle.len())
            .any(|w| w == needle)
        {
            out.push(path.display().to_string());
        }
    }
}

/// Copies a store's files by reading them, as `files_containing` does, skipping the lock file.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().filter_map(Result::ok) {
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == ".hms.lock") {
            continue;
        }
        let target = to.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            std::fs::write(&target, std::fs::read(&path).unwrap()).unwrap();
        }
    }
}

fn generation_dirs(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("vectors_data.bin"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("gen-"))
        .count()
}

fn search(k: u32) -> SearchOptions {
    SearchOptions {
        k: Some(k),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------------------------
// Reference model

#[derive(Clone, Debug)]
struct Fact {
    text: String,
    ingest: Millis,
    valid_from: Option<Millis>,
    valid_to: Option<Millis>,
    event_time: Option<Millis>,
    /// Stored links; forget re-links them, a raw delete leaves them.
    supersedes: BTreeSet<String>,
    /// The input as first written, for idempotent retries.
    spec: Spec,
}

#[derive(Clone, Debug, PartialEq)]
struct Spec {
    text: String,
    valid_from: Option<Millis>,
    valid_to: Option<Millis>,
    event_time: Option<Millis>,
    supersedes: Vec<String>,
}

impl Spec {
    fn input(&self, id: &str) -> RememberInput {
        RememberInput {
            id: Some(id.into()),
            text: self.text.clone(),
            valid_from: self.valid_from,
            valid_to: self.valid_to,
            event_time: self.event_time,
            supersedes: (!self.supersedes.is_empty()).then(|| self.supersedes.clone()),
            ..RememberInput::default()
        }
    }
}

impl Fact {
    fn effective(&self) -> Millis {
        self.valid_from.or(self.event_time).unwrap_or(self.ingest)
    }
}

#[derive(Default, Clone)]
struct Model {
    facts: BTreeMap<String, Fact>,
    docs: BTreeMap<String, String>,
}

impl Model {
    fn texts(&self) -> BTreeMap<String, String> {
        let mut all = self.docs.clone();
        all.extend(
            self.facts
                .iter()
                .map(|(id, f)| (id.clone(), f.text.clone())),
        );
        all
    }

    /// The memory API's status rule (docs/memory-api/README.md#concepts), written out.
    fn status(&self, id: &str, now: Millis, known: Option<Millis>) -> Option<FactStatus> {
        let fact = &self.facts[id];
        if known.is_some_and(|k| fact.ingest > k) {
            return None;
        }
        let superseded = self.facts.values().any(|by| {
            by.supersedes.contains(id)
                && by.effective() <= now
                && known.is_none_or(|k| by.ingest <= k)
        });
        Some(if superseded {
            FactStatus::Superseded
        } else if fact.valid_to.is_some_and(|to| to <= now) {
            FactStatus::Expired
        } else if fact.valid_from.is_some_and(|from| from > now) {
            FactStatus::NotYetValid
        } else {
            FactStatus::Current
        })
    }

    /// Forget without `include_chain`: each survivor that superseded a removed fact
    /// supersedes what that fact did, transitively through removed facts.
    fn forget(&mut self, dead: &BTreeSet<String>) -> bool {
        fn through(model: &Model, id: &str, dead: &BTreeSet<String>, out: &mut BTreeSet<String>) {
            for old in &model.facts[id].supersedes {
                if dead.contains(old) {
                    if out.insert(format!("\u{0}{old}")) {
                        through(model, old, dead, out);
                    }
                } else {
                    out.insert(old.clone());
                }
            }
        }
        let mut bridged = false;
        let survivors: Vec<String> = self
            .facts
            .iter()
            .filter(|(id, f)| !dead.contains(*id) && f.supersedes.iter().any(|s| dead.contains(s)))
            .map(|(id, _)| id.clone())
            .collect();
        for id in survivors {
            let mut out = BTreeSet::new();
            through(self, &id, dead, &mut out);
            let next: BTreeSet<String> = out
                .into_iter()
                .filter(|s| !s.starts_with('\u{0}'))
                .collect();
            let fact = self.facts.get_mut(&id).unwrap();
            if fact.supersedes != next {
                fact.supersedes = next;
                bridged = true;
            }
        }
        for id in dead {
            self.facts.remove(id);
        }
        bridged
    }
}

// ---------------------------------------------------------------------------------------------
// Wall-clock resolution model (docs/memory-api/TIME-TRAVEL.md#as-of-a-wall-clock-time)

#[derive(Clone, Debug)]
struct Anchor {
    before: JournalPosition,
    after: JournalPosition,
    /// The frame's own stamp; the effective time is the running maximum.
    stamp: Millis,
    /// The anchor is the first of two frames (a forget's re-link before its delete), so its
    /// position lies strictly between `before` and `after`.
    inner: bool,
    retained: bool,
}

#[derive(Debug, PartialEq)]
enum Resolved {
    At(JournalPosition, Option<Millis>),
    Inside(JournalPosition, JournalPosition, Millis),
    BeforeRetained,
    /// Only the synthetic prefix of a compacted generation can answer: its frames' stamps are
    /// not modelled, so the answer is one of `BeforeRetained`, `NoTimeAnchor` or the oldest
    /// position (when the prefix's last frame is stamped).
    Prefix,
}

/// The journal generation the store is in, and what of it is retained.
struct Epoch {
    generation: String,
    /// Never compacted: history starts at offset 0 and survives reopen.
    fresh: bool,
    checkpointed: bool,
    oldest: JournalPosition,
    recorded: Vec<(JournalPosition, Snapshot, usize)>,
    /// Bounds of the running maximum carried in by a compaction's synthetic prefix. They
    /// differ only when a background compaction's snapshot point is unknown.
    base: (Option<Millis>, Option<Millis>),
    anchors: Vec<Anchor>,
}

impl Epoch {
    fn resolve(&self, time: Millis, base: Option<Millis>) -> Resolved {
        let mut run = base;
        let mut found = None;
        let mut before_retained = base.is_some_and(|b| b <= time);
        for anchor in &self.anchors {
            let eff = run.map_or(anchor.stamp, |r| r.max(anchor.stamp));
            run = Some(eff);
            if eff <= time {
                if anchor.retained {
                    found = Some((anchor, eff));
                } else {
                    before_retained = true;
                }
            }
        }
        match found {
            Some((a, eff)) if a.inner => Resolved::Inside(a.before.clone(), a.after.clone(), eff),
            Some((a, eff)) => Resolved::At(a.after.clone(), Some(eff)),
            None if self.fresh => Resolved::At(self.oldest.clone(), None),
            None if before_retained && self.anchors.iter().all(|a| a.retained) => Resolved::Prefix,
            None if before_retained => Resolved::BeforeRetained,
            None => Resolved::Prefix,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The live side

#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    listing: Value,
    searches: Value,
    count: usize,
}

const VIEW_QUERIES: [&str; 3] = ["plaindoc", "memfact", "w1q"];

fn snapshot(core: &WarmCore) -> Snapshot {
    Snapshot {
        listing: serde_json::to_value(core.list_documents(None, K as usize).unwrap()).unwrap(),
        searches: Value::Array(
            VIEW_QUERIES
                .iter()
                .map(|q| {
                    serde_json::to_value(core.search_documents(q, &search(K)).unwrap()).unwrap()
                })
                .collect(),
        ),
        count: core.document_count(),
    }
}

struct Sim {
    seed: u64,
    rng: Rng,
    _dir: tempfile::TempDir,
    path: PathBuf,
    config: WarmConfig,
    tiny_checkpoint: bool,
    core: Option<Arc<WarmCore>>,
    memory: Option<Memory>,
    clock: Arc<ManualClock>,
    now: Millis,
    model: Model,
    /// Ids raw-deleted while other facts still link to them; never reused.
    retired: BTreeSet<String>,
    fresh_id: usize,
    marker: usize,
    /// Markers of text no longer live, and whether a foreground compaction has run since.
    dead: BTreeSet<String>,
    epoch: Epoch,
    /// The largest ingest stamp of any frame ever written, for the background-compaction
    /// upper bound.
    max_stamp: Option<Millis>,
    log: Vec<String>,
    step: usize,
    stats: BTreeMap<String, usize>,
}

impl Sim {
    fn new(seed: u64) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store");
        let mut rng = Rng(seed);
        let mut config = WarmConfig::default();
        let tiny_checkpoint = rng.chance(12);
        if tiny_checkpoint {
            config.document_checkpoint.min_journal_bytes = 1;
        }
        let core = Arc::new(
            WarmCore::new(DIM, Some(path.display().to_string()), Some(config.clone())).unwrap(),
        );
        let clock = Arc::new(ManualClock::new(T0));
        let memory = Memory::new(Arc::clone(&core)).with_clock(clock.clone());
        let start = core.journal_position();
        assert_eq!(start.offset, 0);
        let mut sim = Self {
            seed,
            rng,
            _dir: dir,
            path,
            config,
            tiny_checkpoint,
            core: Some(core),
            memory: Some(memory),
            clock,
            now: T0,
            model: Model::default(),
            retired: BTreeSet::new(),
            fresh_id: 0,
            marker: 0,
            dead: BTreeSet::new(),
            epoch: Epoch {
                generation: start.generation.clone(),
                fresh: true,
                checkpointed: false,
                oldest: start.clone(),
                recorded: Vec::new(),
                base: (None, None),
                anchors: Vec::new(),
            },
            max_stamp: None,
            log: Vec::new(),
            step: 0,
            stats: BTreeMap::new(),
        };
        sim.record();
        sim
    }

    fn count(&mut self, what: impl Into<String>) {
        *self.stats.entry(what.into()).or_default() += 1;
    }

    fn core(&self) -> &Arc<WarmCore> {
        self.core.as_ref().unwrap()
    }

    fn memory(&self) -> &Memory {
        self.memory.as_ref().unwrap()
    }

    fn ctx(&self) -> String {
        format!(
            "seed {} step {} (replay with WARM_SIM_SEED={}); ops:\n  {}",
            self.seed,
            self.step,
            self.seed,
            self.log.join("\n  ")
        )
    }

    fn marker(&mut self) -> String {
        self.marker += 1;
        format!("m{}x{}z", self.seed, self.marker)
    }

    fn text(&mut self, kind: &str) -> String {
        let marker = self.marker();
        let mut words: Vec<String> = (0..WORDS)
            .filter(|_| self.rng.chance(45))
            .map(word)
            .collect();
        if words.is_empty() {
            words.push(word(self.rng.below(WORDS)));
        }
        format!("{kind} {marker} {}", words.join(" "))
    }

    fn marker_of(text: &str) -> String {
        text.split(' ').nth(1).unwrap().to_owned()
    }

    fn kill(&mut self, text: &str) {
        self.dead.insert(Self::marker_of(text));
    }

    /// A time near the clock, including the clock itself and the edges of stored intervals.
    fn time(&mut self) -> Millis {
        let mut bank: Vec<Millis> = vec![
            self.now,
            self.now - 1,
            self.now + 1,
            self.now + 7,
            self.now - 9,
            T0 - 1,
        ];
        for f in self.model.facts.values() {
            bank.extend([f.ingest, f.effective()]);
            bank.extend(f.valid_from);
            bank.extend(f.valid_to);
        }
        let t = *self.rng.pick(&bank).unwrap();
        t + [0, 0, 0, -1, 1][self.rng.below(5)]
    }

    fn record(&mut self) {
        let core = Arc::clone(self.core());
        let at = core.journal_position();
        let snap = snapshot(&core);
        if let Some((last, previous, step)) = self.epoch.recorded.last() {
            if *last == at {
                assert_eq!(
                    &snap,
                    previous,
                    "state changed without a journal frame since step {step}; {}",
                    self.ctx()
                );
                return;
            }
        }
        self.epoch.recorded.push((at, snap, self.step));
    }

    fn anchor(&mut self, before: JournalPosition, stamp: Millis, inner: bool) {
        let after = self.core().journal_position();
        assert_ne!(
            before,
            after,
            "a stamped write wrote no frame; {}",
            self.ctx()
        );
        self.max_stamp = Some(self.max_stamp.map_or(stamp, |m| m.max(stamp)));
        self.epoch.anchors.push(Anchor {
            before,
            after,
            stamp,
            inner,
            retained: true,
        });
    }

    // -- operations ---------------------------------------------------------------------------

    fn op(&mut self) {
        let roll = self.rng.below(100);
        match roll {
            0..=7 => self.clock_op(),
            8..=29 => self.remember(),
            30..=34 => self.remember_many(),
            35..=39 => self.retry_or_conflict(),
            40..=46 => self.forget(),
            47..=49 => self.raw_delete_fact(),
            50..=61 => self.put_doc(),
            62..=68 => self.delete_doc(),
            69..=73 => self.compact(),
            74..=83 => self.background_compaction(),
            84..=86 => self.reopen(),
            87..=89 => self.crash(),
            90..=93 => self.flush(),
            _ => self.remember(),
        }
    }

    fn clock_op(&mut self) {
        let delta = [0, 1, 1, 5, 20, -3, -1][self.rng.below(7)];
        self.now += delta;
        self.clock.set(self.now);
        self.log.push(format!("clock {delta:+} -> {}", self.now));
    }

    fn fact_id(&mut self) -> String {
        if self.rng.chance(10) {
            self.fresh_id += 1;
            return format!("fresh{}", self.fresh_id);
        }
        format!("f{}", self.rng.below(FACT_IDS))
    }

    fn spec(&mut self, id: &str, earlier: &[String]) -> (Spec, bool) {
        let text = self.text("memfact");
        let mut ok = true;
        let interval = self.rng.below(10);
        let (mut valid_from, mut valid_to) = (None, None);
        match interval {
            0..=3 => {}
            4 | 5 => valid_from = Some(self.time()),
            6 => valid_to = Some(self.time()),
            7 | 8 => {
                let from = self.time();
                valid_from = Some(from);
                valid_to = Some(from + [1, 2, 10][self.rng.below(3)]);
            }
            _ => {
                // Zero-length or inverted: rejected (validFrom < validTo is required).
                let from = self.time();
                valid_from = Some(from);
                valid_to = Some(from - self.rng.below(2) as Millis);
                ok = false;
            }
        }
        let event_time = self.rng.chance(20).then(|| self.time());
        let mut supersedes = Vec::new();
        if self.rng.chance(45) {
            let live: Vec<String> = self
                .model
                .facts
                .keys()
                .filter(|k| *k != id)
                .cloned()
                .chain(earlier.iter().filter(|e| *e != id).cloned())
                .collect();
            if let Some(old) = self.rng.pick(&live) {
                supersedes.push(old.clone());
            }
            if self.rng.chance(20) {
                if let Some(old) = self.rng.pick(&live) {
                    if !supersedes.contains(old) {
                        supersedes.push(old.clone());
                    }
                }
            }
            if self.rng.chance(8) {
                // An unknown (never written or deleted) fact: rejected.
                let ghost = format!("f{}", FACT_IDS + self.rng.below(3));
                ok &= self.model.facts.contains_key(&ghost);
                supersedes.push(ghost);
            }
        }
        supersedes.sort();
        supersedes.dedup();
        (
            Spec {
                text,
                valid_from,
                valid_to,
                event_time,
                supersedes,
            },
            ok,
        )
    }

    fn fact(&self, spec: &Spec) -> Fact {
        Fact {
            text: spec.text.clone(),
            ingest: self.now,
            valid_from: spec.valid_from,
            valid_to: spec.valid_to,
            event_time: spec.event_time,
            supersedes: spec.supersedes.iter().cloned().collect(),
            spec: spec.clone(),
        }
    }

    fn expect_unchanged(&self, before: &JournalPosition, what: &str) {
        assert_eq!(
            &self.core().journal_position(),
            before,
            "{what} wrote a frame; {}",
            self.ctx()
        );
    }

    fn remember(&mut self) {
        let id = self.fact_id();
        if self.retired.contains(&id) {
            return;
        }
        if self.model.facts.contains_key(&id) {
            return self.retry_or_conflict_on(id);
        }
        let (spec, ok) = self.spec(&id, &[]);
        self.log.push(format!("remember {id} {spec:?}"));
        let before = self.core().journal_position();
        let result = self.memory().remember(spec.input(&id));
        match (ok, result) {
            (true, Ok(r)) => {
                assert!(r.created, "{}", self.ctx());
                assert_eq!(r.ingest_time, self.now, "{}", self.ctx());
                let fact = self.fact(&spec);
                self.model.facts.insert(id, fact);
                self.anchor(before, self.now, false);
            }
            (false, Err(_)) => self.expect_unchanged(&before, "a rejected remember"),
            (ok, result) => panic!("remember expected ok={ok}, got {result:?}; {}", self.ctx()),
        }
    }

    fn remember_many(&mut self) {
        let n = 1 + self.rng.below(3);
        let mut ids: Vec<String> = Vec::new();
        let mut specs = Vec::new();
        let mut ok = true;
        for _ in 0..n {
            let id = self.fact_id();
            if ids.contains(&id) || self.retired.contains(&id) || self.model.facts.contains_key(&id)
            {
                continue;
            }
            let (spec, valid) = self.spec(&id, &ids);
            ok &= valid;
            ids.push(id);
            specs.push(spec);
        }
        if ids.is_empty() {
            return;
        }
        self.log.push(format!("remember_many {ids:?} {specs:?}"));
        let before = self.core().journal_position();
        let inputs = ids.iter().zip(&specs).map(|(id, s)| s.input(id)).collect();
        match (ok, self.memory().remember_many(inputs)) {
            (true, Ok(r)) => {
                assert!(r.iter().all(|r| r.created), "{}", self.ctx());
                for (id, spec) in ids.into_iter().zip(&specs) {
                    let fact = self.fact(spec);
                    self.model.facts.insert(id, fact);
                }
                self.anchor(before, self.now, false);
            }
            (false, Err(_)) => self.expect_unchanged(&before, "a rejected batch"),
            (ok, result) => panic!(
                "remember_many expected ok={ok}, got {result:?}; {}",
                self.ctx()
            ),
        }
    }

    fn retry_or_conflict(&mut self) {
        let ids: Vec<String> = self.model.facts.keys().cloned().collect();
        if let Some(id) = self.rng.pick(&ids).cloned() {
            self.retry_or_conflict_on(id);
        }
    }

    /// The same payload under a live id is idempotent and keeps the first ingest time; a
    /// different payload is a conflict. Neither writes a frame.
    fn retry_or_conflict_on(&mut self, id: String) {
        let fact = self.model.facts[&id].clone();
        let before = self.core().journal_position();
        if self.rng.chance(60) {
            self.log.push(format!("retry {id}"));
            let r = self
                .memory()
                .remember(fact.spec.input(&id))
                .unwrap_or_else(|e| panic!("retry failed: {e:#}; {}", self.ctx()));
            assert!(!r.created, "{}", self.ctx());
            assert_eq!(r.ingest_time, fact.ingest, "{}", self.ctx());
            self.expect_unchanged(&before, "an idempotent retry");
        } else {
            self.log.push(format!("conflict {id}"));
            let mut spec = fact.spec.clone();
            spec.text.push_str(" changed");
            let error = self
                .memory()
                .remember(spec.input(&id))
                .err()
                .unwrap_or_else(|| panic!("conflict accepted; {}", self.ctx()));
            assert!(
                matches!(
                    error.downcast_ref::<MemoryError>(),
                    Some(MemoryError::Conflict(_))
                ),
                "{error:#}; {}",
                self.ctx()
            );
            self.expect_unchanged(&before, "a conflicting write");
        }
    }

    fn forget(&mut self) {
        let mut ids: BTreeSet<String> = BTreeSet::new();
        for _ in 0..1 + self.rng.below(2) {
            // Sometimes a fact that is not there (never written or already forgotten).
            ids.insert(self.fact_id());
        }
        ids.retain(|id| !self.retired.contains(id));
        if ids.is_empty() {
            return;
        }
        let purge = self.rng.chance(10);
        self.log.push(format!("forget {ids:?} purge={purge}"));
        if purge {
            self.check_views();
        }
        let before = self.core().journal_position();
        let report = self
            .memory()
            .forget(
                &ForgetScope::Ids {
                    ids: ids.iter().cloned().collect(),
                },
                &ForgetOptions {
                    purge,
                    ..Default::default()
                },
            )
            .unwrap_or_else(|e| panic!("forget: {e:#}; {}", self.ctx()));
        let dead: BTreeSet<String> = ids
            .into_iter()
            .filter(|id| self.model.facts.contains_key(id))
            .collect();
        assert_eq!(
            report.forgotten.iter().cloned().collect::<BTreeSet<_>>(),
            dead,
            "{}",
            self.ctx()
        );
        for id in &dead {
            let text = self.model.facts[id].text.clone();
            self.kill(&text);
        }
        let survivors_ingest = self
            .model
            .facts
            .iter()
            .filter(|(id, f)| !dead.contains(*id) && f.supersedes.iter().any(|s| dead.contains(s)))
            .map(|(_, f)| f.ingest)
            .max();
        let bridged = self.model.forget(&dead);
        if dead.is_empty() {
            if !purge {
                self.expect_unchanged(&before, "forgetting nothing");
            }
        } else if bridged && !purge {
            self.count("bridged forget");
            self.anchor(before, survivors_ingest.unwrap(), true);
        }
        if purge && !dead.is_empty() {
            self.after_publication(false);
            self.scan_files();
        }
    }

    /// `delete_document` on a fact: no re-linking, so a survivor keeps a link to the id; the
    /// id is never reused.
    fn raw_delete_fact(&mut self) {
        let ids: Vec<String> = self.model.facts.keys().cloned().collect();
        let Some(id) = self.rng.pick(&ids).cloned() else {
            return;
        };
        self.log.push(format!("raw delete fact {id}"));
        let hash_before = self.hash();
        let record = self.core().get_document(&id).unwrap().unwrap();
        assert!(self.core().delete_document(&id).unwrap(), "{}", self.ctx());
        assert_eq!(
            self.hash(),
            hash_before.wrapping_sub(Self::record_hash(&serde_json::to_value(record).unwrap())),
            "a delete changed another record; {}",
            self.ctx()
        );
        let text = self.model.facts.remove(&id).unwrap().text;
        self.kill(&text);
        if self
            .model
            .facts
            .values()
            .any(|f| f.supersedes.contains(&id))
        {
            self.retired.insert(id);
        }
    }

    fn put_doc(&mut self) {
        let id = format!("d{}", self.rng.below(DOC_IDS));
        let text = self.text("plaindoc");
        self.log.push(format!("put {id} {text:?}"));
        let before = self.core().journal_position();
        self.core()
            .memorize_document(
                serde_json::from_value::<DocumentInput>(json!({"id": id, "text": text})).unwrap(),
            )
            .unwrap_or_else(|e| panic!("put: {e:#}; {}", self.ctx()));
        assert_ne!(before, self.core().journal_position(), "{}", self.ctx());
        if let Some(old) = self.model.docs.insert(id, text) {
            self.kill(&old);
        }
    }

    fn delete_doc(&mut self) {
        // Includes ids never written and ids already deleted.
        let id = format!("d{}", self.rng.below(DOC_IDS + 1));
        self.log.push(format!("delete {id}"));
        let hash_before = self.hash();
        let record = self.core().get_document(&id).unwrap();
        let existed = self.core().delete_document(&id).unwrap();
        assert_eq!(existed, self.model.docs.contains_key(&id), "{}", self.ctx());
        match record {
            Some(record) => {
                assert_eq!(
                    self.hash(),
                    hash_before
                        .wrapping_sub(Self::record_hash(&serde_json::to_value(record).unwrap())),
                    "a delete changed another record; {}",
                    self.ctx()
                );
                let text = self.model.docs.remove(&id).unwrap();
                self.kill(&text);
            }
            // A missing id still commits a no-op delete frame (`delete_document` logs
            // unconditionally); the state must not change.
            None => assert_eq!(hash_before, self.hash(), "{}", self.ctx()),
        }
    }

    fn compact(&mut self) {
        self.log.push("compact".into());
        self.check_views();
        let hash = self.hash();
        self.core()
            .compact()
            .unwrap_or_else(|e| panic!("compact: {e:#}; {}", self.ctx()));
        assert_eq!(
            hash,
            self.hash(),
            "compaction changed the logical content; {}",
            self.ctx()
        );
        self.after_publication(false);
        self.scan_files();
    }

    fn background_compaction(&mut self) {
        let options = CompactionOptions {
            require_reclaim: self.rng.chance(50),
            max_cpu_fraction: if self.rng.chance(20) { 0.5 } else { 1.0 },
            ..Default::default()
        };
        let tail = self.rng.below(4);
        let cancel = self.rng.chance(25);
        let second = self.rng.chance(15);
        self.log.push(format!(
            "background compaction {options:?} tail={tail} cancel={cancel} second={second}"
        ));
        self.check_views();
        let hash = self.hash();
        let max_before = self.max_stamp;
        let generation = self.core().journal_position().generation;
        let dirs = generation_dirs(&self.path);
        let handle = self.core().start_compaction(options.clone()).unwrap();
        let other = second.then(|| self.core().start_compaction(options).unwrap());
        for _ in 0..tail {
            match self.rng.below(4) {
                0 => self.remember(),
                1 => self.put_doc(),
                2 => self.delete_doc(),
                _ => self.forget_quiet(),
            }
        }
        if cancel {
            handle.cancel();
        }
        let report = handle.wait();
        drop(handle);
        let other = other.map(|h| h.wait());
        let tail_written = self.max_stamp != max_before || tail > 0;
        for r in std::iter::once(&report).chain(other.as_ref()) {
            assert!(
                matches!(
                    r.status,
                    CompactionStatus::Completed
                        | CompactionStatus::Cancelled
                        | CompactionStatus::Busy
                        | CompactionStatus::NoProgress
                ),
                "{r:?}; {}",
                self.ctx()
            );
            assert_eq!(
                r.published,
                r.status == CompactionStatus::Completed,
                "{r:?}; {}",
                self.ctx()
            );
            if cancel && std::ptr::eq(r, &report) && r.status == CompactionStatus::Completed {
                assert!(r.progress.cancel_requested, "{r:?}");
            }
        }
        for r in std::iter::once(&report).chain(other.as_ref()) {
            self.count(format!("background {:?} tail={}", r.status, tail > 0));
        }
        let published = report.published || other.as_ref().is_some_and(|r| r.published);
        // Every outcome leaves exactly the live generation (none before the first compaction).
        assert_eq!(
            generation_dirs(&self.path),
            if published { 1 } else { dirs },
            "{report:?} {other:?}; {}",
            self.ctx()
        );
        let now_generation = self.core().journal_position().generation;
        assert_eq!(
            published,
            now_generation != generation,
            "{report:?}; {}",
            self.ctx()
        );
        if tail == 0 {
            assert_eq!(
                hash,
                self.hash(),
                "compaction changed the logical content; {}",
                self.ctx()
            );
        }
        if published {
            self.after_publication(tail_written);
            if tail == 0 {
                self.scan_files();
            }
        }
        self.check_live();
    }

    /// A forget inside a background compaction's tail; recorded positions there are folded
    /// into the compaction's prefix.
    fn forget_quiet(&mut self) {
        self.forget_no_purge();
    }

    fn forget_no_purge(&mut self) {
        let ids: Vec<String> = self.model.facts.keys().cloned().collect();
        let Some(id) = self.rng.pick(&ids).cloned() else {
            return;
        };
        self.log.push(format!("forget {id} (tail)"));
        let before = self.core().journal_position();
        let dead = BTreeSet::from([id.clone()]);
        let survivors_ingest = self
            .model
            .facts
            .values()
            .filter(|f| f.supersedes.contains(&id))
            .map(|f| f.ingest)
            .max();
        self.memory()
            .forget(
                &ForgetScope::Ids {
                    ids: vec![id.clone()],
                },
                &ForgetOptions::default(),
            )
            .unwrap_or_else(|e| panic!("forget: {e:#}; {}", self.ctx()));
        let text = self.model.facts[&id].text.clone();
        self.kill(&text);
        if self.model.forget(&dead) {
            self.count("bridged forget");
            self.anchor(before, survivors_ingest.unwrap(), true);
        }
    }

    /// A compaction published: positions of the old generation are refused, the new one's
    /// history starts here, and the synthetic prefix carries the live facts' stamps.
    fn after_publication(&mut self, uncertain: bool) {
        let core = Arc::clone(self.core());
        let live_max = self.model.facts.values().map(|f| f.ingest).max();
        let hi = if uncertain { self.max_stamp } else { live_max };
        let bounds = match core.history_bounds() {
            Ok(bounds) => bounds,
            Err(e) => {
                // A compaction of a store with no vector state (here: no documents, perhaps
                // only at a background snapshot) writes a document checkpoint, and such a
                // generation has no replayable history (TIME-TRAVEL.md).
                assert!(
                    self.tiny_checkpoint || uncertain || self.model.texts().is_empty(),
                    "bounds: {e:#}; {}",
                    self.ctx()
                );
                assert_eq!(
                    history_error(e),
                    HistoryError::CheckpointBase,
                    "{}",
                    self.ctx()
                );
                let at = core.journal_position();
                assert_ne!(at.generation, self.epoch.generation, "{}", self.ctx());
                self.count("checkpoint generation");
                self.epoch = Epoch {
                    generation: at.generation.clone(),
                    fresh: false,
                    checkpointed: true,
                    oldest: at,
                    recorded: Vec::new(),
                    base: (live_max, hi.max(live_max)),
                    anchors: Vec::new(),
                };
                return;
            }
        };
        assert_ne!(
            bounds.oldest.generation,
            self.epoch.generation,
            "{}",
            self.ctx()
        );
        // Tail writes that land after the publication extend the new generation's journal, so
        // only a compaction without a tail leaves the history at its first position.
        assert_eq!(bounds.latest, core.journal_position(), "{}", self.ctx());
        assert_eq!(
            bounds.oldest.generation,
            bounds.latest.generation,
            "{}",
            self.ctx()
        );
        if !uncertain {
            assert_eq!(bounds.latest, bounds.oldest, "{}", self.ctx());
        }
        for (at, _, _) in self.epoch.recorded.iter().take(3) {
            assert!(
                matches!(
                    history_error(core.history_view(at).err().unwrap()),
                    HistoryError::GenerationNotRetained { .. }
                ),
                "{}",
                self.ctx()
            );
        }
        self.epoch = Epoch {
            generation: bounds.oldest.generation.clone(),
            fresh: false,
            checkpointed: false,
            oldest: bounds.oldest.clone(),
            recorded: Vec::new(),
            base: (live_max, hi.max(live_max)),
            anchors: Vec::new(),
        };
        self.record();
    }

    fn reopen(&mut self) {
        self.log.push("reopen".into());
        self.check_views();
        let hash = self.hash();
        let before = self.core().journal_position();
        drop(self.memory.take());
        let core = self.core.take().unwrap();
        drop(Arc::try_unwrap(core).ok().expect("the engine is shared"));
        let core = Arc::new(
            WarmCore::new(
                DIM,
                Some(self.path.display().to_string()),
                Some(self.config.clone()),
            )
            .unwrap_or_else(|e| panic!("reopen: {e:#}; {}", self.ctx())),
        );
        self.memory = Some(Memory::new(Arc::clone(&core)).with_clock(self.clock.clone()));
        self.core = Some(core);
        assert_eq!(
            hash,
            self.hash(),
            "reopen changed the logical content; {}",
            self.ctx()
        );
        self.after_maintenance(before, true);
    }

    /// Opens a copy of the files as they are while the engine still runs, the state a killed
    /// process leaves (no close, no close-time checkpoint), and continues on the copy.
    fn crash(&mut self) {
        self.log.push("crash".into());
        self.check_views();
        let hash = self.hash();
        let before = self.core().journal_position();
        let copy = tempfile::tempdir().unwrap();
        let target = copy.path().join("store");
        copy_tree(&self.path, &target);
        drop(self.memory.take());
        drop(self.core.take());
        let core = Arc::new(
            WarmCore::new(
                DIM,
                Some(target.display().to_string()),
                Some(self.config.clone()),
            )
            .unwrap_or_else(|e| panic!("open after crash: {e:#}; {}", self.ctx())),
        );
        self._dir = copy;
        self.path = target;
        self.memory = Some(Memory::new(Arc::clone(&core)).with_clock(self.clock.clone()));
        self.core = Some(core);
        assert_eq!(
            hash,
            self.hash(),
            "a crash changed the logical content; {}",
            self.ctx()
        );
        self.after_maintenance(before, true);
    }

    fn flush(&mut self) {
        self.log.push("flush".into());
        self.check_views();
        let before = self.core().journal_position();
        self.core().flush().unwrap();
        self.after_maintenance(before, false);
    }

    /// After a reopen or flush: a never-compacted journal keeps its whole history; a
    /// compacted one restarts it at the end; a document checkpoint ends replayable history.
    fn after_maintenance(&mut self, before: JournalPosition, reopened: bool) {
        let core = Arc::clone(self.core());
        match core.history_bounds() {
            Err(e) => {
                assert!(
                    self.tiny_checkpoint || self.epoch.checkpointed,
                    "{e:#}; {}",
                    self.ctx()
                );
                assert_eq!(
                    history_error(e),
                    HistoryError::CheckpointBase,
                    "{}",
                    self.ctx()
                );
                assert_eq!(
                    history_error(core.history_view(&core.journal_position()).err().unwrap()),
                    HistoryError::CheckpointBase
                );
                assert!(core.resolve_history_time(self.now).is_err());
                self.epoch.checkpointed = true;
                self.epoch.recorded.clear();
            }
            Ok(bounds) => {
                assert_eq!(bounds.latest, core.journal_position(), "{}", self.ctx());
                if bounds.oldest.generation != self.epoch.generation {
                    assert!(self.tiny_checkpoint, "{bounds:?}; {}", self.ctx());
                    self.after_publication(false);
                } else if self.epoch.checkpointed {
                    panic!(
                        "a checkpointed generation became replayable; {}",
                        self.ctx()
                    );
                } else if self.epoch.fresh || !reopened {
                    assert_eq!(bounds.oldest, self.epoch.oldest, "{}", self.ctx());
                } else {
                    assert_eq!(bounds.oldest, before, "{}", self.ctx());
                    if bounds.oldest != self.epoch.oldest {
                        let first = self.epoch.recorded[0].0.clone();
                        assert!(
                            matches!(
                                history_error(core.history_view(&first).err().unwrap()),
                                HistoryError::BeforeRetained { .. }
                            ),
                            "{}",
                            self.ctx()
                        );
                        self.epoch.oldest = bounds.oldest.clone();
                        self.epoch
                            .recorded
                            .retain(|(at, _, _)| *at == bounds.oldest);
                        let oldest = bounds.oldest.offset;
                        for a in &mut self.epoch.anchors {
                            a.retained = !a.inner && a.after.offset >= oldest;
                        }
                    }
                }
            }
        }
        self.record();
    }

    // -- checks -------------------------------------------------------------------------------

    fn record_hash(record: &Value) -> u128 {
        record_hash(&[&record.to_string()])
    }

    /// The control total over every live record (id, version, metadata, text, source, chunk
    /// count: all logical; nothing in a record names a slot or generation).
    fn hash(&self) -> u128 {
        self.core()
            .list_documents(None, K as usize)
            .unwrap()
            .into_iter()
            .map(|r| Self::record_hash(&serde_json::to_value(r).unwrap()))
            .fold(0u128, u128::wrapping_add)
    }

    fn check_live(&mut self) {
        let core = Arc::clone(self.core());
        let texts = self.model.texts();
        let listing: BTreeMap<String, String> = core
            .list_documents(None, K as usize)
            .unwrap()
            .into_iter()
            .map(|r| (r.id, r.text.unwrap_or_default()))
            .collect();
        assert_eq!(listing, texts, "listing; {}", self.ctx());
        assert_eq!(core.document_count(), texts.len(), "count; {}", self.ctx());
        let model_total = texts
            .iter()
            .map(|(id, text)| record_hash(&[id, text]))
            .fold(0u128, u128::wrapping_add);
        let live_total = listing
            .iter()
            .map(|(id, text)| record_hash(&[id, text]))
            .fold(0u128, u128::wrapping_add);
        assert_eq!(model_total, live_total, "control total; {}", self.ctx());

        for query in ["plaindoc", "memfact", &word(self.rng.below(WORDS))] {
            let found: BTreeSet<String> = core
                .search_documents(query, &search(K))
                .unwrap()
                .into_iter()
                .map(|r| r.document_id)
                .collect();
            let expected: BTreeSet<String> = texts
                .iter()
                .filter(|(_, t)| t.split(' ').any(|w| w == query))
                .map(|(id, _)| id.clone())
                .collect();
            assert_eq!(found, expected, "search {query:?}; {}", self.ctx());
        }

        let mut times = vec![(None, None)];
        for _ in 0..2 {
            let as_of = self.time();
            let known = self.rng.chance(50).then(|| self.time());
            times.push((Some(as_of), known));
        }
        for (as_of, known) in times {
            let now = as_of.unwrap_or(self.now);
            let recalled = self
                .memory()
                .recall(
                    "memfact",
                    &RecallOptions {
                        k: Some(K),
                        as_of,
                        known_at: known,
                        history: Some(HistoryMode::All),
                        ..Default::default()
                    },
                )
                .unwrap_or_else(|e| panic!("recall: {e:#}; {}", self.ctx()));
            for s in recalled.iter().map(|r| r.status) {
                self.count(format!("status {s:?}"));
            }
            let got: BTreeMap<String, FactStatus> =
                recalled.into_iter().map(|r| (r.id, r.status)).collect();
            let expected: BTreeMap<String, FactStatus> = self
                .model
                .facts
                .keys()
                .filter_map(|id| self.model.status(id, now, known).map(|s| (id.clone(), s)))
                .collect();
            assert_eq!(
                got,
                expected,
                "recall as_of {as_of:?} known_at {known:?}; {}",
                self.ctx()
            );
            let current: BTreeSet<String> = self
                .memory()
                .recall(
                    "memfact",
                    &RecallOptions {
                        k: Some(K),
                        as_of,
                        known_at: known,
                        ..Default::default()
                    },
                )
                .unwrap()
                .into_iter()
                .map(|r| r.id)
                .collect();
            let expected_current: BTreeSet<String> = expected
                .iter()
                .filter(|(_, s)| **s == FactStatus::Current)
                .map(|(id, _)| id.clone())
                .collect();
            assert_eq!(
                current,
                expected_current,
                "current recall as_of {as_of:?} known_at {known:?}; {}",
                self.ctx()
            );
        }
        if !self.epoch.checkpointed {
            self.check_time_resolution();
        }
    }

    fn check_time_resolution(&mut self) {
        let core = Arc::clone(self.core());
        for _ in 0..3 {
            let t = self.time();
            let lo = self.epoch.resolve(t, self.epoch.base.0);
            let hi = self.epoch.resolve(t, self.epoch.base.1);
            if lo != hi {
                continue;
            }
            self.count(format!(
                "resolve {}",
                format!("{lo:?}").split('(').next().unwrap()
            ));
            let got = core.resolve_history_time(t);
            match (&lo, got) {
                (Resolved::At(at, anchor), Ok(r)) => {
                    assert_eq!(
                        (&r.position, &r.anchor_ms),
                        (at, anchor),
                        "resolve {t}; {}",
                        self.ctx()
                    );
                }
                (Resolved::Inside(before, after, eff), Ok(r)) => {
                    assert_eq!(r.position.generation, after.generation, "{}", self.ctx());
                    assert!(
                        before.offset < r.position.offset && r.position.offset < after.offset,
                        "resolve {t} gave {:?}, expected inside {before}..{after}; {}",
                        r.position,
                        self.ctx()
                    );
                    assert_eq!(r.anchor_ms, Some(*eff), "{}", self.ctx());
                }
                (Resolved::BeforeRetained, Err(e)) => assert!(
                    matches!(history_error(e), HistoryError::BeforeRetained { .. }),
                    "resolve {t}; {}",
                    self.ctx()
                ),
                (Resolved::Prefix, Err(e)) => assert!(
                    matches!(
                        history_error(e),
                        HistoryError::BeforeRetained { .. } | HistoryError::NoTimeAnchor { .. }
                    ),
                    "resolve {t}; {}",
                    self.ctx()
                ),
                (Resolved::Prefix, Ok(r)) => {
                    assert_eq!(r.position, self.epoch.oldest, "resolve {t}; {}", self.ctx());
                    assert!(
                        r.anchor_ms.is_some_and(|a| a <= t),
                        "resolve {t}: {r:?}; {}",
                        self.ctx()
                    );
                }
                (expected, got) => panic!(
                    "resolve {t}: expected {expected:?}, got {got:?}; {}",
                    self.ctx()
                ),
            }
        }
    }

    /// Every recorded position of the live generation replays to the state recorded there.
    fn check_views(&mut self) {
        if self.epoch.checkpointed {
            return;
        }
        let core = Arc::clone(self.core());
        let bounds = core.history_bounds().unwrap();
        assert_eq!(bounds.oldest, self.epoch.oldest, "{}", self.ctx());
        *self.stats.entry("view".into()).or_default() += self.epoch.recorded.len();
        for (at, snap, step) in &self.epoch.recorded {
            let view = core
                .history_view(at)
                .unwrap_or_else(|e| panic!("view at {at} (step {step}): {e:#}; {}", self.ctx()));
            let listing =
                serde_json::to_value(view.list_documents(None, K as usize).unwrap()).unwrap();
            assert_eq!(
                &listing,
                &snap.listing,
                "view listing at {at} (step {step}); {}",
                self.ctx()
            );
            assert_eq!(
                view.document_count().unwrap(),
                snap.count,
                "view count at {at}; {}",
                self.ctx()
            );
            let searches = Value::Array(
                VIEW_QUERIES
                    .iter()
                    .map(|q| {
                        serde_json::to_value(view.search_documents(q, &search(K)).unwrap()).unwrap()
                    })
                    .collect(),
            );
            assert_eq!(
                &searches,
                &snap.searches,
                "view search at {at} (step {step}); {}",
                self.ctx()
            );
        }
        let revisions = core
            .history_revisions(&bounds.oldest, &bounds.latest, 100_000)
            .unwrap_or_else(|e| panic!("revisions: {e:#}; {}", self.ctx()));
        let open: BTreeSet<String> = revisions
            .iter()
            .filter(|r| r.ended.is_none())
            .map(|r| r.id.clone())
            .collect();
        let live: BTreeSet<String> = self.model.texts().into_keys().collect();
        assert_eq!(open, live, "revisions open at the end; {}", self.ctx());
    }

    /// After a published compaction, no file holds a dead document's text, and every live
    /// document's text is still found (so the scan is not vacuous).
    fn scan_files(&mut self) {
        let live: Vec<String> = self
            .model
            .texts()
            .values()
            .map(|t| Self::marker_of(t))
            .collect();
        for marker in &self.dead {
            let mut found = Vec::new();
            files_containing(&self.path, marker.as_bytes(), &mut found);
            assert!(
                found.is_empty(),
                "dead text {marker} in {found:?}; {}",
                self.ctx()
            );
        }
        if let Some(marker) = live.first() {
            let mut found = Vec::new();
            files_containing(&self.path, marker.as_bytes(), &mut found);
            assert!(
                !found.is_empty(),
                "live text {marker} not on disk; {}",
                self.ctx()
            );
        }
    }

    fn run(mut self) -> BTreeMap<String, usize> {
        for step in 0..OPS {
            self.step = step;
            self.op();
            self.check_live();
            self.record();
        }
        self.check_views();
        self.log.push("final compact".into());
        self.compact();
        self.reopen();
        self.check_live();
        self.scan_files();
        std::mem::take(&mut self.stats)
    }
}

fn run_seeds(seeds: impl Iterator<Item = u64>) {
    if let Some(seed) = std::env::var("WARM_SIM_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        Sim::new(seed).run();
        return;
    }
    let mut total: BTreeMap<String, usize> = BTreeMap::new();
    for seed in seeds {
        for (what, n) in Sim::new(seed).run() {
            *total.entry(what).or_default() += n;
        }
    }
    // What the seeds exercised; shown with --nocapture.
    eprintln!("{total:#?}");
}

// Eight shards of 25 seeds run in parallel; the time goes to journal syncs, not CPU.
#[test]
fn simulation_seeds_0_to_24() {
    run_seeds(0..25);
}

#[test]
fn simulation_seeds_25_to_49() {
    run_seeds(25..50);
}

#[test]
fn simulation_seeds_50_to_74() {
    run_seeds(50..75);
}

#[test]
fn simulation_seeds_75_to_99() {
    run_seeds(75..100);
}

#[test]
fn simulation_seeds_100_to_124() {
    run_seeds(100..125);
}

#[test]
fn simulation_seeds_125_to_149() {
    run_seeds(125..150);
}

#[test]
fn simulation_seeds_150_to_174() {
    run_seeds(150..175);
}

#[test]
fn simulation_seeds_175_to_199() {
    run_seeds(175..200);
}

#[test]
#[ignore = "long run: WARM_SIM_SEEDS=<count> cargo test --test simulation -- --ignored"]
fn simulation_long_run() {
    let count = std::env::var("WARM_SIM_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5_000u64);
    // Seeds are independent; workers share them so a slow seed does not idle the rest.
    let next = std::sync::atomic::AtomicU64::new(1_000);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| loop {
                let seed = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if seed >= 1_000 + count {
                    break;
                }
                Sim::new(seed).run();
            });
        }
    });
}

// ---------------------------------------------------------------------------------------------
// Deterministic gates

fn j<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap()
}

fn open_store(path: &Path) -> Arc<WarmCore> {
    Arc::new(WarmCore::new(DIM, Some(path.display().to_string()), None).unwrap())
}

fn statuses(memory: &Memory, as_of: Millis) -> BTreeMap<String, FactStatus> {
    memory
        .recall(
            "memfact",
            &RecallOptions {
                k: Some(K),
                as_of: Some(as_of),
                history: Some(HistoryMode::All),
                ..Default::default()
            },
        )
        .unwrap()
        .into_iter()
        .map(|r| (r.id, r.status))
        .collect()
}

/// Deleting the newest version, by `forget` or by a raw `delete_document`, makes the version
/// it superseded current again, exactly as stored, in live recall at every time, in the view
/// after the delete, and after compaction and reopen; the deleted text leaves no file.
#[test]
fn deleting_a_superseder_restores_the_older_fact() {
    for raw in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store");
        let core = open_store(&path);
        let clock = Arc::new(ManualClock::new(T0));
        let memory = Memory::new(Arc::clone(&core)).with_clock(clock.clone());
        const OLD: &str = "oldmarkerq";
        const NEW: &str = "newmarkerq";
        memory
            .remember(RememberInput {
                id: Some("a".into()),
                text: format!("memfact {OLD} lives in paris"),
                valid_from: Some(100),
                ..Default::default()
            })
            .unwrap();
        let record = j(&core.get_document("a").unwrap().unwrap());
        let fact = j(&memory.get("a").unwrap().unwrap());
        let before_b = core.journal_position();
        memory
            .remember(RememberInput {
                id: Some("b".into()),
                text: format!("memfact {NEW} lives in rome"),
                valid_from: Some(200),
                supersedes: Some(vec!["a".into()]),
                ..Default::default()
            })
            .unwrap();
        let with_b = core.journal_position();
        let both = |a: FactStatus, b: FactStatus| {
            BTreeMap::from([("a".to_string(), a), ("b".to_string(), b)])
        };
        assert_eq!(
            statuses(&memory, 199),
            both(FactStatus::Current, FactStatus::NotYetValid)
        );
        assert_eq!(
            statuses(&memory, 200),
            both(FactStatus::Superseded, FactStatus::Current)
        );

        if raw {
            assert!(core.delete_document("b").unwrap());
        } else {
            let report = memory
                .forget(
                    &ForgetScope::Ids {
                        ids: vec!["b".into()],
                    },
                    &ForgetOptions::default(),
                )
                .unwrap();
            assert_eq!(report.forgotten, vec!["b".to_string()]);
        }
        let after = core.journal_position();
        let only_a = BTreeMap::from([("a".to_string(), FactStatus::Current)]);
        let check = |core: &WarmCore, memory: &Memory| {
            assert_eq!(
                j(&core.get_document("a").unwrap().unwrap()),
                record,
                "raw={raw}"
            );
            assert_eq!(j(&memory.get("a").unwrap().unwrap()), fact, "raw={raw}");
            assert!(memory.get("b").unwrap().is_none());
            for t in [100, 199, 200, 201, i64::MAX] {
                assert_eq!(statuses(memory, t), only_a, "as_of {t} raw={raw}");
            }
            assert_eq!(
                j(&memory.history("a").unwrap()),
                Value::Array(vec![fact.clone()])
            );
        };
        check(&core, &memory);
        let view = core.history_view(&after).unwrap();
        assert_eq!(j(&view.get_document("a").unwrap().unwrap()), record);
        assert!(view.get_document("b").unwrap().is_none());
        // A delete is history: the position before it still shows the superseder.
        assert!(core
            .history_view(&with_b)
            .unwrap()
            .get_document("b")
            .unwrap()
            .is_some());
        assert_eq!(
            j(&core
                .history_view(&before_b)
                .unwrap()
                .get_document("a")
                .unwrap()
                .unwrap()),
            record
        );
        drop(view);

        core.compact().unwrap();
        check(&core, &memory);
        let scan = |needle: &str| {
            let mut found = Vec::new();
            files_containing(&path, needle.as_bytes(), &mut found);
            found
        };
        assert!(scan(NEW).is_empty(), "raw={raw}: {:?}", scan(NEW));
        assert!(!scan(OLD).is_empty(), "the scan found no live text");
        drop(memory);
        drop(Arc::try_unwrap(core).ok().unwrap());
        let core = open_store(&path);
        let memory = Memory::new(Arc::clone(&core)).with_clock(clock);
        check(&core, &memory);
        assert!(scan(NEW).is_empty(), "raw={raw} after reopen");
    }
}

/// Result and revision limits at limit - 1, limit and limit + 1, live and in a view.
#[test]
fn result_and_revision_limits_hold_at_their_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let core = open_store(&dir.path().join("store"));
    let n = K as usize + 1;
    core.memorize_documents(
        (0..n)
            .map(|i| {
                serde_json::from_value(
                    json!({"id": format!("d{i:04}"), "text": format!("common w{}q", i % 7)}),
                )
                .unwrap()
            })
            .collect(),
    )
    .unwrap();
    let at = core.journal_position();
    let view = core.history_view(&at).unwrap();
    for k in [K - 1, K] {
        let live = core.search_documents("common", &search(k)).unwrap();
        assert_eq!(live.len(), k as usize);
        let historical = view.search_documents("common", &search(k)).unwrap();
        assert_eq!(
            serde_json::to_value(&live).unwrap(),
            serde_json::to_value(&historical).unwrap()
        );
    }
    assert!(core.search_documents("common", &search(K + 1)).is_err());
    assert!(view.search_documents("common", &search(K + 1)).is_err());
    assert_eq!(view.document_count().unwrap(), n);
    let memory = Memory::new(Arc::clone(&core)).with_clock(Arc::new(ManualClock::new(T0)));
    let recall = |k| {
        memory.recall(
            "common",
            &RecallOptions {
                k: Some(k),
                ..Default::default()
            },
        )
    };
    assert_eq!(recall(K).unwrap().len(), K as usize);
    assert!(recall(K + 1).is_err());

    let bounds = core.history_bounds().unwrap();
    let revisions = |limit| core.history_revisions(&bounds.oldest, &bounds.latest, limit);
    assert!(matches!(
        history_error(revisions(n - 1).err().unwrap()),
        HistoryError::TooManyRevisions { .. }
    ));
    assert_eq!(revisions(n).unwrap().len(), n);
    assert_eq!(revisions(n + 1).unwrap().len(), n);
}
