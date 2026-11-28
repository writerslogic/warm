// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! The derived tables in a checkpoint are gated by the store's manifest: a store created by
//! this build records `derived_tables` and its checkpoints carry them; a store whose manifest
//! lacks the field keeps the older checkpoint encoding (an older build still opens it) however
//! it is written; `warm-format upgrade` writes a new store with the field.

use std::path::{Path, PathBuf};
use writerslogic_warm::core::documents::{
    DocumentInput, EncodeMode, IngestOptions, LexicalExecution, LexicalOptions, LexicalQuery,
};
use writerslogic_warm::core::format::upgrade_store;
use writerslogic_warm::core::WarmConfig;
use writerslogic_warm::{BatchOp, WarmCore};

const DIM: u32 = 256;
/// The checkpoint header's feature word and its bit for the derived tables.
const FEATURES_AT: usize = 8 + 8 * 3;
const FEATURE_DERIVED: u64 = 2;
/// Header word of the derived section's offset and length (the last of the 13 sections).
const DERIVED_AT: usize = 8 + 8 * (14 + 2 * 12);
/// The derived section's layout this build writes. A build that knows only layout 1 finds a
/// header it does not match, computes its tables and reads none of the newer bytes.
const DERIVED_FORMAT: u64 = 2;

fn input(id: &str, text: &str) -> DocumentInput {
    serde_json::from_value(serde_json::json!({ "id": id, "text": text })).unwrap()
}

fn config() -> WarmConfig {
    let mut config = WarmConfig::unencrypted();
    config.document_checkpoint.min_journal_bytes = 0;
    config
}

fn open(dir: &Path) -> WarmCore {
    WarmCore::new(DIM, Some(dir.display().to_string()), Some(config())).unwrap()
}

/// Lexical-only documents: the only ones a document checkpoint holds.
fn lexical() -> IngestOptions {
    IngestOptions {
        encode: EncodeMode::Lexical,
        ..Default::default()
    }
}

fn seed(core: &WarmCore, from: usize, to: usize) {
    for n in from..to {
        core.memorize_document_with(
            input(&format!("d{n}"), &format!("alpha w{} w{}", n % 9, n % 4)),
            &lexical(),
        )
        .unwrap();
    }
}

fn checkpoints(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .file_name()
                .is_some_and(|name| name == "documents.ckpt")
            {
                found.push(path);
            }
        }
    }
    found
}

fn features(path: &Path) -> u64 {
    let bytes = std::fs::read(path).unwrap();
    u64::from_le_bytes(bytes[FEATURES_AT..FEATURES_AT + 8].try_into().unwrap())
}

/// The derived section's format word, its slot-norm count and its weighted-list count.
fn derived_header(path: &Path) -> (u64, u64, u64) {
    let bytes = std::fs::read(path).unwrap();
    let word = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize;
    let (start, len) = (word(DERIVED_AT), word(DERIVED_AT + 8));
    assert!(len > 15 * 8 && start + len == bytes.len());
    (
        word(start) as u64,
        word(start + 12 * 8) as u64,
        word(start + 13 * 8) as u64,
    )
}

fn manifest(dir: &Path) -> Vec<u8> {
    std::fs::read(dir.join("vectors_data.bin").join("format.json")).unwrap()
}

fn count(core: &WarmCore) -> usize {
    let options = LexicalOptions {
        k: 1,
        count: true,
        ..Default::default()
    };
    let result = core
        .search_lexical(&LexicalQuery::parse("alpha").unwrap(), &options)
        .unwrap();
    result.count.unwrap()
}

/// A store as an older build created it: the manifest without the field, so every checkpoint
/// is written without derived tables.
fn old_store(dir: &Path) {
    open(dir).close().unwrap();
    let bytes = {
        let mut value: serde_json::Value = serde_json::from_slice(&manifest(dir)).unwrap();
        value.as_object_mut().unwrap().remove("derived_tables");
        serde_json::to_vec_pretty(&value).unwrap()
    };
    let path = dir.join("vectors_data.bin").join("format.json");
    std::fs::write(&path, &bytes).unwrap();
    std::fs::write(
        path.with_file_name("format.json.crc32"),
        format!("{:08x}\n", crc32fast::hash(&bytes)),
    )
    .unwrap();
    let core = open(dir);
    seed(&core, 0, 60);
    core.compact().unwrap();
    core.close().unwrap();
}

#[test]
fn a_new_store_records_the_field_and_its_checkpoint_carries_the_tables() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path());
    seed(&core, 0, 60);
    core.compact().unwrap();
    core.close().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&manifest(dir.path())).unwrap();
    assert_eq!(value["derived_tables"], true);
    let found = checkpoints(dir.path());
    assert!(!found.is_empty());
    for path in found {
        assert_eq!(features(&path) & FEATURE_DERIVED, FEATURE_DERIVED);
        let (format, norms, _) = derived_header(&path);
        assert_eq!(format, DERIVED_FORMAT);
        assert_eq!(norms, 60, "one stored norm per slot of a small store");
    }
}

#[test]
fn an_old_store_is_never_converted_by_writing_to_it() {
    let dir = tempfile::tempdir().unwrap();
    old_store(dir.path());
    let before = manifest(dir.path());
    assert!(!checkpoints(dir.path()).is_empty());
    for path in checkpoints(dir.path()) {
        assert_eq!(features(&path) & FEATURE_DERIVED, 0);
    }
    let core = open(dir.path());
    core.memorize_document_with(input("extra", "alpha extra"), &lexical())
        .unwrap();
    core.delete_document("d3").unwrap();
    core.write_batch_with(
        vec![
            BatchOp::Memorize {
                document: input("batch", "alpha batch"),
            },
            BatchOp::Delete { id: "d4".into() },
        ],
        &lexical(),
    )
    .unwrap();
    core.compact().unwrap();
    assert_eq!(count(&core), 60 + 2 - 2);
    core.close().unwrap();
    assert_eq!(manifest(dir.path()), before, "the manifest is unchanged");
    for path in checkpoints(dir.path()) {
        assert_eq!(features(&path) & FEATURE_DERIVED, 0, "{path:?}");
    }
    let reader =
        WarmCore::open_read_only(DIM, Some(dir.path().display().to_string()), Some(config()))
            .unwrap();
    assert_eq!(count(&reader), 60);
    reader.close().unwrap();
    assert_eq!(manifest(dir.path()), before);
}

#[test]
fn upgrade_leaves_the_old_store_and_writes_a_new_one_with_the_field() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("old");
    std::fs::create_dir(&source).unwrap();
    old_store(&source);
    let before = manifest(&source);
    let destination = dir.path().join("new");
    upgrade_store(&source, &destination, false).unwrap();
    assert_eq!(manifest(&source), before);
    let value: serde_json::Value = serde_json::from_slice(&manifest(&destination)).unwrap();
    assert_eq!(value["derived_tables"], true);
    // The copy has the flag while its checkpoint still has no tables: it serves from the
    // non-table path, ranking like the exhaustive scan, until its next compaction writes them.
    let carried = checkpoints(&destination);
    assert!(!carried.is_empty());
    for path in &carried {
        assert_eq!(features(path) & FEATURE_DERIVED, 0);
    }
    let core = open(&destination);
    assert_eq!(count(&core), 60);
    for query in ["alpha", "alpha w3", "+alpha -w4", "w1 w2 w3"] {
        let parsed = LexicalQuery::parse(query).unwrap();
        for k in [3usize, 10, 100] {
            let options = LexicalOptions {
                k,
                count: true,
                ..Default::default()
            };
            let exhaustive = LexicalOptions {
                execution: LexicalExecution::Exhaustive,
                ..options
            };
            assert_eq!(
                core.search_lexical(&parsed, &options).unwrap(),
                core.search_lexical(&parsed, &exhaustive).unwrap(),
                "{query:?} k={k}"
            );
        }
    }
    core.compact().unwrap();
    core.close().unwrap();
    for path in checkpoints(&destination) {
        assert_eq!(features(&path) & FEATURE_DERIVED, FEATURE_DERIVED);
    }
}

/// A compacted store of the size the unpruned ranker serves answers bare disjunctions from its
/// stored norms and weights: the hits, scores to the bit and the count are the exhaustive
/// ones, at depths around the weighing block (128), for lists on both sides of it.
#[test]
fn a_small_store_ranks_disjunctions_as_the_exhaustive_scan() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path());
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for n in 0..1200usize {
        let words: Vec<String> = (0..3 + next() % 20)
            .map(|_| format!("w{}", next() % (1 + next() % 80)))
            .chain((n < 127).then(|| "edge127".to_owned()))
            .chain((n < 128).then(|| "edge128".to_owned()))
            .chain((n < 129).then(|| "edge129".to_owned()))
            .collect();
        core.memorize_document_with(input(&format!("d{n}"), &words.join(" ")), &lexical())
            .unwrap();
    }
    core.compact().unwrap();
    core.close().unwrap();
    let core = open(dir.path());
    let queries = [
        "edge127",
        "edge128",
        "edge129",
        "edge127 edge128 edge129",
        "w1 w2",
        "w1 w2 w3 w4 w5 w6 w7",
        "w3 edge129 w9",
        "w70 w71 w72",
        "w0",
    ];
    for query in queries {
        let parsed = LexicalQuery::parse(query).unwrap();
        for k in [1usize, 10, 127, 128, 129, 1000] {
            for count in [false, true] {
                let options = LexicalOptions {
                    k,
                    count,
                    ..Default::default()
                };
                let exhaustive = LexicalOptions {
                    execution: LexicalExecution::Exhaustive,
                    ..options
                };
                let got = core.search_lexical(&parsed, &options).unwrap();
                let want = core.search_lexical(&parsed, &exhaustive).unwrap();
                assert_eq!(got.count, want.count, "{query:?} k={k}");
                assert_eq!(got.hits.len(), want.hits.len(), "{query:?} k={k}");
                for (a, b) in got.hits.iter().zip(&want.hits) {
                    assert_eq!(
                        (&a.document_id, a.chunk_index, a.score.to_bits()),
                        (&b.document_id, b.chunk_index, b.score.to_bits()),
                        "{query:?} k={k}"
                    );
                }
            }
        }
    }
    core.close().unwrap();
}
