// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Historical views over the journal (docs/memory-api/TIME-TRAVEL.md): the state at every
//! earlier position equals a store that only ever received the writes before it, purged text
//! is never returned, and positions outside the retained history fail closed.

use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};
use writerslogic_warm::core::memory_api::*;
use writerslogic_warm::core::WarmConfig;
use writerslogic_warm::{
    DocumentInput, EmbeddingSpace, EntangledHVec, HistoryError, JournalPosition, RevisionKind,
    SearchOptions, WarmCore,
};

const DIM: u32 = 1024;
const QUERIES: [&str; 5] = [
    "ownership threads",
    "holographic binding trace",
    "frost tomatoes roses",
    "revised edition",
    "batch",
];

fn config() -> WarmConfig {
    WarmConfig {
        embedding_space: Some(EmbeddingSpace {
            model: "fixture".into(),
            revision: "1".into(),
            dimensions: 3,
            normalization: "l2".into(),
            metric: "cosine".into(),
        }),
        ..Default::default()
    }
}

fn open(path: &Path) -> WarmCore {
    WarmCore::new(DIM, Some(path.display().to_string()), Some(config())).unwrap()
}

fn doc(value: Value) -> DocumentInput {
    serde_json::from_value(value).unwrap()
}

type Step = fn(&WarmCore);

/// Writes, updates and deletes of documents and raw vectors, one transaction each (the
/// batch is one transaction of two documents).
fn steps() -> Vec<Step> {
    vec![
        |c| {
            c.memorize_document(doc(json!({"id": "a", "text": "Rust ownership rules prevent data races across threads", "metadata": {"n": 1}})))
                .map(drop)
                .unwrap()
        },
        |c| {
            c.memorize_document(doc(json!({"id": "b", "text": "Holographic reduced representations bind vectors into one trace", "embeddings": [[0.1, 0.9, 0.2]]})))
                .map(drop)
                .unwrap()
        },
        |c| {
            c.memorize_document(doc(
                json!({"id": "c", "text": "Prune roses and plant tomatoes after the last frost"}),
            ))
            .map(drop)
            .unwrap()
        },
        |c| {
            c.memorize_document(doc(json!({"id": "b", "text": "Revised edition: holographic binding with circular convolution", "version": "2", "embeddings": [[0.8, 0.1, 0.3]]})))
                .map(drop)
                .unwrap()
        },
        |c| assert!(c.delete_document("a").unwrap()),
        |c| {
            c.memorize(
                "v".into(),
                EntangledHVec::new_deterministic(DIM as usize, 7),
            )
            .unwrap()
        },
        |c| {
            c.memorize_documents(vec![
                doc(json!({"id": "d", "text": "A batch document about ownership and frost"})),
                doc(json!({"id": "e", "text": "Another batch document, revised and bound"})),
            ])
            .map(drop)
            .unwrap()
        },
        |c| {
            c.memorize(
                "v".into(),
                EntangledHVec::new_deterministic(DIM as usize, 8),
            )
            .unwrap()
        },
        |c| assert!(c.delete_document("c").unwrap()),
        |c| assert!(c.delete("v").unwrap()),
        |c| {
            c.memorize_document(doc(
                json!({"id": "a", "text": "Ownership returns in a new revision", "version": "3"}),
            ))
            .map(drop)
            .unwrap()
        },
    ]
}

/// Everything a view and a live store both answer, rendered for exact comparison.
struct Observed {
    listing: Value,
    gets: Value,
    searches: Value,
    vector: Option<Vec<u32>>,
    count: usize,
}

const IDS: [&str; 6] = ["a", "b", "c", "d", "e", "missing"];

fn search_options() -> [SearchOptions; 2] {
    [
        SearchOptions::default(),
        SearchOptions {
            embedding: Some(vec![0.7, 0.2, 0.3]),
            ..Default::default()
        },
    ]
}

/// The raw vector `v` after the first `applied` steps (the engine has no public getter for
/// one raw vector, so the oracle follows the steps: set at 5 and 7, deleted at 9).
fn expected_vector(applied: usize) -> Option<Vec<u32>> {
    let seed = match applied {
        6..=7 => 7,
        8..=9 => 8,
        _ => return None,
    };
    Some(
        EntangledHVec::new_deterministic(DIM as usize, seed)
            .indices()
            .to_vec(),
    )
}

fn observe_live(core: &WarmCore, applied: usize) -> Observed {
    let mut searches = Vec::new();
    for query in QUERIES {
        for options in search_options() {
            searches.push(
                serde_json::to_value(core.search_documents(query, &options).unwrap()).unwrap(),
            );
        }
    }
    Observed {
        listing: serde_json::to_value(core.list_documents(None, 100).unwrap()).unwrap(),
        gets: Value::Array(
            IDS.iter()
                .map(|id| serde_json::to_value(core.get_document(id).unwrap()).unwrap())
                .collect(),
        ),
        searches: Value::Array(searches),
        vector: expected_vector(applied),
        count: core.document_count(),
    }
}

fn observe_view(core: &WarmCore, at: &JournalPosition) -> Observed {
    let view = core.history_view(at).unwrap();
    assert_eq!(view.position(), at);
    let mut searches = Vec::new();
    for query in QUERIES {
        for options in search_options() {
            searches.push(
                serde_json::to_value(view.search_documents(query, &options).unwrap()).unwrap(),
            );
        }
    }
    let vector = view.get_vector("v").unwrap().map(|v| v.indices().to_vec());
    let ids = view.vector_ids().unwrap();
    assert_eq!(ids.is_empty(), vector.is_none());
    Observed {
        listing: serde_json::to_value(view.list_documents(None, 100).unwrap()).unwrap(),
        gets: Value::Array(
            IDS.iter()
                .map(|id| serde_json::to_value(view.get_document(id).unwrap()).unwrap())
                .collect(),
        ),
        searches: Value::Array(searches),
        vector,
        count: view.document_count().unwrap(),
    }
}

fn assert_same(view: &Observed, live: &Observed, when: &str) {
    assert_eq!(view.listing, live.listing, "{when}: listing");
    assert_eq!(view.gets, live.gets, "{when}: get_document");
    assert_eq!(view.searches, live.searches, "{when}: search_documents");
    assert_eq!(view.vector, live.vector, "{when}: vector");
    assert_eq!(view.count, live.count, "{when}: count");
}

fn history_error(error: anyhow::Error) -> HistoryError {
    error
        .downcast::<HistoryError>()
        .unwrap_or_else(|e| panic!("not a history error: {e:#}"))
}

/// Runs the steps on one store, recording the position after each, and returns it with the
/// positions (index 0 is the empty start).
fn written(path: &Path) -> (WarmCore, Vec<JournalPosition>) {
    let core = open(path);
    let mut positions = vec![core.journal_position()];
    assert_eq!(positions[0].offset, 0);
    for step in steps() {
        step(&core);
        positions.push(core.journal_position());
    }
    (core, positions)
}

/// The issue's done-when: at every earlier position the view gives exactly what a store
/// that received only the writes before it gives, for listing, get, lexical and embedding
/// search, and the raw vector.
#[test]
fn every_earlier_position_matches_a_store_that_stopped_there() {
    let dir = tempfile::tempdir().unwrap();
    let (core, positions) = written(&dir.path().join("history"));
    let steps = steps();
    assert!(positions.windows(2).all(|w| w[0].offset < w[1].offset));
    for (i, at) in positions.iter().enumerate() {
        let oracle_dir = dir.path().join(format!("oracle-{i}"));
        let oracle = open(&oracle_dir);
        for step in &steps[..i] {
            step(&oracle);
        }
        assert_same(
            &observe_view(&core, at),
            &observe_live(&oracle, i),
            &format!("position {i}"),
        );
    }
    // The current position is the current state.
    let last = positions.last().unwrap();
    assert_eq!(&core.journal_position(), last);
    assert_same(
        &observe_view(&core, last),
        &observe_live(&core, steps.len()),
        "current",
    );
    // Spot checks against the writes themselves.
    let at = |i: usize| core.history_view(&positions[i]).unwrap();
    assert_eq!(at(0).document_count().unwrap(), 0);
    assert_eq!(
        at(1).get_document("a").unwrap().unwrap().text.as_deref(),
        Some("Rust ownership rules prevent data races across threads")
    );
    assert!(at(4)
        .get_document("b")
        .unwrap()
        .unwrap()
        .text
        .unwrap()
        .starts_with("Revised"));
    assert!(at(3)
        .get_document("b")
        .unwrap()
        .unwrap()
        .text
        .unwrap()
        .starts_with("Holographic"));
    assert!(at(4).get_document("a").unwrap().is_some());
    assert!(at(5).get_document("a").unwrap().is_none());
    assert!(at(11)
        .get_document("a")
        .unwrap()
        .unwrap()
        .text
        .unwrap()
        .starts_with("Ownership returns"));
    // A reopened, never-compacted store keeps the whole history.
    drop(core);
    let reopened = open(&dir.path().join("history"));
    let bounds = reopened.history_bounds().unwrap();
    assert_eq!(bounds.oldest, positions[0]);
    assert_eq!(&bounds.latest, last);
    assert_same(
        &observe_view(&reopened, &positions[4]),
        &{
            let oracle = open(&dir.path().join("oracle-4"));
            observe_live(&oracle, 4)
        },
        "position 4 after reopen",
    );
}

/// First, last and one-past-the-end positions; an offset inside a frame; a foreign
/// generation.
#[test]
fn positions_outside_the_journal_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let (core, positions) = written(&dir.path().join("store"));
    let first = &positions[1];
    assert_eq!(
        core.history_view(first).unwrap().document_count().unwrap(),
        1
    );
    let last = positions.last().unwrap();
    assert_eq!(
        core.history_view(last).unwrap().document_count().unwrap(),
        4
    );
    let past = JournalPosition {
        offset: last.offset + 1,
        ..last.clone()
    };
    assert!(matches!(
        history_error(core.history_view(&past).err().unwrap()),
        HistoryError::PastEnd { .. }
    ));
    for offset in [1, first.offset - 1, first.offset + 1, last.offset - 1] {
        let inside = JournalPosition {
            offset,
            ..last.clone()
        };
        assert!(
            matches!(
                history_error(core.history_view(&inside).err().unwrap()),
                HistoryError::NotFrameBoundary { .. }
            ),
            "offset {offset}"
        );
    }
    let foreign = JournalPosition {
        generation: "gen-1-1".into(),
        offset: 0,
    };
    assert!(matches!(
        history_error(core.history_view(&foreign).err().unwrap()),
        HistoryError::GenerationNotRetained { .. }
    ));
    // Interval endpoints are checked the same way.
    assert!(matches!(
        history_error(
            core.history_revisions(&positions[3], &past, 100)
                .err()
                .unwrap()
        ),
        HistoryError::PastEnd { .. }
    ));
    assert!(matches!(
        history_error(
            core.history_revisions(&positions[3], &positions[2], 100)
                .err()
                .unwrap()
        ),
        HistoryError::IntervalInverted { .. }
    ));
}

/// A revision as (kind, id, version, written offset, ended offset, deleted).
type Revision = (
    RevisionKind,
    String,
    Option<String>,
    Option<u64>,
    Option<u64>,
    bool,
);

/// Revisions live at some position of `[from, to]`, with where each began and ended.
#[test]
fn interval_lists_each_revision_live_in_it() {
    let dir = tempfile::tempdir().unwrap();
    let (core, p) = written(&dir.path().join("store"));
    let summary = |from: usize, to: usize| -> Vec<Revision> {
        core.history_revisions(&p[from], &p[to], 100)
            .unwrap()
            .into_iter()
            .map(|r| {
                (
                    r.kind,
                    r.id,
                    r.version,
                    r.written.map(|w| w.offset),
                    r.ended.map(|e| e.offset),
                    r.deleted,
                )
            })
            .collect()
    };
    let doc = RevisionKind::Document;
    let vec = RevisionKind::Vector;
    let v1 = core
        .history_view(&p[1])
        .unwrap()
        .get_document("a")
        .unwrap()
        .unwrap()
        .version;
    let v2 = core
        .history_view(&p[2])
        .unwrap()
        .get_document("b")
        .unwrap()
        .unwrap()
        .version;
    let v3 = core
        .history_view(&p[3])
        .unwrap()
        .get_document("c")
        .unwrap()
        .unwrap()
        .version;
    let s = |x: &str| Some(x.to_string());
    let o = |i: usize| Some(p[i].offset);
    // [p3, p6]: a until its delete at p5; b's first revision until p4, then its second;
    // c throughout; the vector from p6.
    assert_eq!(
        summary(3, 6),
        vec![
            (doc, "a".into(), s(&v1), o(1), o(5), true),
            (doc, "b".into(), s(&v2), o(2), o(4), false),
            (doc, "b".into(), s("2"), o(4), None, false),
            (doc, "c".into(), s(&v3), o(3), None, false),
            (vec, "v".into(), None, o(6), None, false),
        ]
    );
    // A one-position interval is the state there.
    let at7 = summary(7, 7);
    let view_ids: Vec<String> = core
        .history_view(&p[7])
        .unwrap()
        .list_documents(None, 100)
        .unwrap()
        .into_iter()
        .map(|d| d.id)
        .collect();
    let listed: Vec<String> = at7
        .iter()
        .filter(|r| r.0 == doc)
        .map(|r| r.1.clone())
        .collect();
    assert_eq!(listed, view_ids);
    assert!(at7.iter().all(|r| r.4.is_none()));
    // The whole journal.
    let all = summary(0, 11);
    assert_eq!(all.iter().filter(|r| r.0 == vec).count(), 2);
    assert_eq!(all.iter().filter(|r| r.1 == "a").count(), 2);
    assert!(all.iter().any(|r| r.0 == vec && r.4 == o(10) && r.5));
    // Too many revisions is an error, not a truncated list.
    assert!(matches!(
        history_error(core.history_revisions(&p[0], &p[11], 3).err().unwrap()),
        HistoryError::TooManyRevisions { limit: 3 }
    ));
    assert_eq!(core.history_revisions(&p[0], &p[0], 1).unwrap(), vec![]);
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

fn mentions(core: &WarmCore, at: &JournalPosition, marker: &str) -> bool {
    let view = core.history_view(at).unwrap();
    let listed = serde_json::to_string(&view.list_documents(None, 1000).unwrap()).unwrap();
    let found = serde_json::to_string(
        &view
            .search_documents(marker, &SearchOptions::default())
            .unwrap(),
    )
    .unwrap();
    listed.contains(marker)
        || found.contains(marker)
        || view.get_document("secret").unwrap().is_some()
}

/// A delete is history; a purge is not. After `forget` with `purge`, no position returns the
/// purged fact, positions from before the purge are refused, and a view built before it
/// stops answering.
#[test]
fn purged_text_is_never_returned_at_any_position() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store");
    let core = Arc::new(open(&path));
    let memory = Memory::new(Arc::clone(&core)).with_clock(Arc::new(ManualClock::new(10)));
    let remember = |id: &str, text: &str| {
        memory
            .remember(RememberInput {
                id: Some(id.into()),
                text: text.into(),
                embedding: Some(vec![0.3, 0.3, 0.9]),
                ..RememberInput::default()
            })
            .unwrap();
    };
    const MARKER: &str = "zyxquartzmarker";
    remember("keep", "an ordinary fact about gardens");
    remember("secret", &format!("the passphrase is {MARKER}"));
    let with_secret = core.journal_position();
    remember("later", "another ordinary fact");
    let before_forget = core.journal_position();
    // A plain delete leaves history: earlier positions still show it.
    memory
        .forget(
            &ForgetScope::Ids {
                ids: vec!["secret".into()],
            },
            &ForgetOptions {
                purge: false,
                ..ForgetOptions::default()
            },
        )
        .unwrap();
    let after_delete = core.journal_position();
    assert!(mentions(&core, &with_secret, MARKER));
    assert!(mentions(&core, &before_forget, MARKER));
    assert!(!mentions(&core, &after_delete, MARKER));
    let held = core.history_view(&with_secret).unwrap();
    assert!(held.get_document("secret").unwrap().is_some());

    // Purge (the delete is already in the journal; forgetting again with purge compacts).
    remember("secret", &format!("the passphrase is {MARKER}"));
    let report = memory
        .forget(
            &ForgetScope::Ids {
                ids: vec!["secret".into()],
            },
            &ForgetOptions {
                purge: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(report.purged);
    let mut leaked = Vec::new();
    files_containing(&path, MARKER.as_bytes(), &mut leaked);
    assert!(leaked.is_empty(), "purge left the marker in {leaked:?}");

    // The view built before the purge refuses every read.
    for result in [
        held.get_document("secret").map(drop),
        held.list_documents(None, 10).map(drop),
        held.search_documents(MARKER, &SearchOptions::default())
            .map(drop),
        held.document_count().map(drop),
        held.vector_ids().map(drop),
    ] {
        assert_eq!(
            history_error(result.err().unwrap()),
            HistoryError::Compacted
        );
    }
    // Every pre-purge position is refused.
    for at in [&with_secret, &before_forget, &after_delete] {
        assert!(matches!(
            history_error(core.history_view(at).err().unwrap()),
            HistoryError::GenerationNotRetained { .. }
        ));
    }
    // The new generation's synthetic prefix is not retained; from its oldest retained
    // position on, nothing shows the marker, before and after more writes and a reopen.
    let bounds = core.history_bounds().unwrap();
    assert_ne!(bounds.oldest.generation, with_secret.generation);
    assert!(bounds.oldest.offset > 0);
    let zero = JournalPosition {
        offset: 0,
        ..bounds.oldest.clone()
    };
    assert!(matches!(
        history_error(core.history_view(&zero).err().unwrap()),
        HistoryError::BeforeRetained { .. }
    ));
    let mut positions = vec![bounds.oldest.clone()];
    remember("after", "a fact written after the purge");
    positions.push(core.journal_position());
    core.delete_document("keep").unwrap();
    positions.push(core.journal_position());
    for at in &positions {
        assert!(!mentions(&core, at, MARKER), "{at}");
    }
    assert!(core
        .history_view(&positions[1])
        .unwrap()
        .get_document("keep")
        .unwrap()
        .is_some());
    assert!(core
        .history_view(&positions[2])
        .unwrap()
        .get_document("keep")
        .unwrap()
        .is_none());
    let revisions = core
        .history_revisions(&positions[0], &positions[2], 100)
        .unwrap();
    assert!(revisions.iter().all(|r| r.id != "secret"));
    // Revisions from the compacted prefix carry no synthetic position.
    assert!(revisions
        .iter()
        .any(|r| r.id == "keep" && r.written.is_none() && r.deleted));
    drop(memory);
    drop(held);
    drop(Arc::try_unwrap(core).ok().unwrap());
    let reopened = open(&path);
    let reopened_bounds = reopened.history_bounds().unwrap();
    assert_eq!(reopened_bounds.oldest, *positions.last().unwrap());
    assert!(!mentions(&reopened, &reopened_bounds.latest, MARKER));
    assert!(matches!(
        history_error(reopened.history_view(&positions[0]).err().unwrap()),
        HistoryError::BeforeRetained { .. }
    ));
}

/// Wall-clock times map through the running maximum of memory-API ingest stamps; unstamped
/// frames after the anchor are excluded; a clock stepping back cannot reorder history.
#[test]
fn wall_clock_times_follow_the_ingest_high_water_mark() {
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(open(&dir.path().join("store")));
    let clock = Arc::new(ManualClock::new(100));
    let memory = Memory::new(Arc::clone(&core)).with_clock(clock.clone());
    let remember = |id: &str| {
        memory
            .remember(RememberInput {
                id: Some(id.into()),
                text: format!("fact {id}"),
                ..RememberInput::default()
            })
            .unwrap();
        core.journal_position()
    };
    let start = core.journal_position();
    let p100 = remember("f100");
    core.memorize_document(doc(json!({"id": "plain", "text": "an unstamped document"})))
        .unwrap();
    let unstamped = core.journal_position();
    clock.set(200);
    let p200 = remember("f200");
    clock.set(150); // the clock steps back
    let p150 = remember("f150");
    clock.set(300);
    let p300 = remember("f300");
    let resolve = |t: i64| core.resolve_history_time(t).unwrap();
    assert_eq!(resolve(50).position, start);
    assert_eq!(resolve(50).anchor_ms, None);
    assert_eq!(resolve(99).position, start);
    assert_eq!(resolve(100).position, p100);
    assert_eq!(resolve(100).anchor_ms, Some(100));
    // The unstamped document is not proven written by 199.
    assert_eq!(resolve(199).position, p100);
    assert_ne!(resolve(199).position, unstamped);
    // The fact stamped 150 after one stamped 200 takes effect at 200.
    assert_eq!(resolve(150).position, p100);
    assert_eq!(resolve(200).position, p150);
    assert_eq!(resolve(200).anchor_ms, Some(200));
    assert_eq!(resolve(299).position, p150);
    assert_eq!(resolve(i64::MAX).position, p300);
    let view = core.history_view_at_time(250, &Default::default()).unwrap();
    assert_eq!(view.position(), &p150);
    assert!(view.get_document("plain").unwrap().is_some());
    assert!(view.get_document("f300").unwrap().is_none());
    assert_ne!(p200, p150);

    // A compacted generation has no anchor before its oldest retained position.
    core.compact().unwrap();
    assert!(matches!(
        history_error(core.resolve_history_time(50).err().unwrap()),
        HistoryError::NoTimeAnchor { time_ms: 50 }
    ));
    assert!(matches!(
        history_error(core.resolve_history_time(250).err().unwrap()),
        HistoryError::BeforeRetained { .. }
    ));
    clock.set(400);
    let p400 = remember("f400");
    assert_eq!(core.resolve_history_time(400).unwrap().position, p400);
}

fn find_segment(dir: &Path) -> Option<std::path::PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_segment(&path) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|name| name == "seg_0.bin") {
            return Some(path);
        }
    }
    None
}

/// A journal frame that no longer passes its checks fails the view; nothing partial is
/// returned and the live store keeps answering.
#[test]
fn a_damaged_frame_fails_the_view() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store");
    let (core, positions) = written(&path);
    let segment = find_segment(&path).expect("the journal's first segment");
    let original = std::fs::read(&segment).unwrap();
    // A byte of the second frame's payload (the first frame ends at positions[1]).
    let at = positions[1].offset as usize + 20;
    let mut damaged = original.clone();
    damaged[at] ^= 0xff;
    {
        use std::io::{Seek, SeekFrom, Write};
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(&segment)
            .unwrap();
        file.seek(SeekFrom::Start(at as u64)).unwrap();
        file.write_all(&damaged[at..=at]).unwrap();
        file.sync_all().unwrap();
    }
    assert!(core.history_view(&positions[1]).is_ok());
    for later in &positions[2..] {
        let error = core.history_view(later).err().unwrap();
        assert!(error.downcast_ref::<HistoryError>().is_none(), "{error:#}");
    }
    assert!(core
        .history_revisions(&positions[0], positions.last().unwrap(), 100)
        .is_err());
    assert_eq!(core.document_count(), 4);
    {
        use std::io::{Seek, SeekFrom, Write};
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(&segment)
            .unwrap();
        file.seek(SeekFrom::Start(at as u64)).unwrap();
        file.write_all(&original[at..=at]).unwrap();
        file.sync_all().unwrap();
    }
    assert!(core.history_view(positions.last().unwrap()).is_ok());
}

/// The state bound is enforced.
#[test]
fn the_state_bound_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let (core, positions) = written(&dir.path().join("store"));
    let options = writerslogic_warm::HistoryOptions {
        max_state_bytes: 512,
    };
    assert!(matches!(
        history_error(
            core.history_view_with(positions.last().unwrap(), &options)
                .err()
                .unwrap()
        ),
        HistoryError::StateTooLarge { limit: 512 }
    ));
    assert!(core.history_view_with(&positions[0], &options).is_ok());
}
