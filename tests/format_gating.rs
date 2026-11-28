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
