// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! A mixed `write_batch` meets the tables derived from the index: the list summaries the
//! write keeps and the tables a compaction stores. Every ranked answer equals the exhaustive
//! one on the live store, after compaction, and on a reader opened after the batch.

use std::path::Path;
use writerslogic_warm::core::documents::{
    DocumentInput, LexicalExecution, LexicalOptions, LexicalQuery,
};
use writerslogic_warm::core::WarmConfig;
use writerslogic_warm::{BatchOp, WarmCore};

const DIM: u32 = 256;
/// Documents holding "zebra": more than 64 blocks of 128 postings, so its list keeps a summary.
const DOCS: usize = 9_000;

fn input(id: &str, text: &str) -> DocumentInput {
    serde_json::from_value(serde_json::json!({ "id": id, "text": text })).unwrap()
}

fn text(n: usize) -> String {
    let mut out = format!("zebra w{} w{}", n % 40, n % 7);
    if n.is_multiple_of(3) {
        out += " common";
    }
    if n.is_multiple_of(11) {
        out += " zebra zebra rare";
    }
    out
}

fn open(dir: &Path) -> WarmCore {
    WarmCore::new(
        DIM,
        Some(dir.display().to_string()),
        Some(WarmConfig::unencrypted()),
    )
    .unwrap()
}

fn open_reader(dir: &Path) -> WarmCore {
    WarmCore::open_read_only(
        DIM,
        Some(dir.display().to_string()),
        Some(WarmConfig::unencrypted()),
    )
    .unwrap()
}

/// Every query ranks the same under the planner's choice and the exhaustive scan.
fn assert_exhaustive(core: &WarmCore, label: &str) {
    for query in [
        "zebra",
        "zebra common",
        "+zebra +common",
        "zebra rare w3",
        "\"zebra zebra\"",
        "+zebra -common",
        "w1 w2 w3",
    ] {
        let parsed = LexicalQuery::parse(query).unwrap();
        for k in [3usize, 10, 100] {
            let options = LexicalOptions {
                k,
                count: true,
                ..Default::default()
            };
            let planned = core.search_lexical(&parsed, &options).unwrap();
            let exhaustive = core
                .search_lexical(
                    &parsed,
                    &LexicalOptions {
                        execution: LexicalExecution::Exhaustive,
                        ..options
                    },
                )
                .unwrap();
            assert_eq!(planned, exhaustive, "{label}: {query:?} k={k}");
        }
    }
}

fn batch() -> Vec<BatchOp> {
    let mut ops = Vec::new();
    for n in 0..400 {
        let id = format!("d{}", n * 17 % DOCS);
        ops.push(match n % 4 {
            0 => BatchOp::Delete { id },
            1 => BatchOp::Memorize {
                document: input(&id, &format!("{} zebra replaced", text(n + 1))),
            },
            2 => BatchOp::Memorize {
                document: input(&format!("new{n}"), &text(n)),
            },
            _ => BatchOp::Delete {
                id: format!("new{}", n - 1),
            },
        });
    }
    ops
}

#[test]
fn a_mixed_batch_ranks_like_the_exhaustive_scan_live_compacted_and_read() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path());
    for start in (0..DOCS).step_by(3_000) {
        core.memorize_documents(
            (start..(start + 3_000).min(DOCS))
                .map(|n| input(&format!("d{n}"), &text(n)))
                .collect(),
        )
        .unwrap();
    }
    assert_exhaustive(&core, "seeded");
    core.write_batch(batch()).unwrap();
    assert_exhaustive(&core, "after the batch");
    let reader = open_reader(dir.path());
    assert_exhaustive(&reader, "reader after the batch");
    let expected: Vec<_> = {
        let parsed = LexicalQuery::parse("zebra common").unwrap();
        let options = LexicalOptions {
            k: 20,
            count: true,
            ..Default::default()
        };
        vec![core.search_lexical(&parsed, &options).unwrap()]
    };
    reader.close().unwrap();
    core.compact().unwrap();
    assert_exhaustive(&core, "after compaction");
    let parsed = LexicalQuery::parse("zebra common").unwrap();
    let options = LexicalOptions {
        k: 20,
        count: true,
        ..Default::default()
    };
    assert_eq!(core.search_lexical(&parsed, &options).unwrap(), expected[0]);
    core.write_batch(batch()).unwrap();
    assert_exhaustive(&core, "a second batch over the checkpoint");
    core.compact().unwrap();
    let reader = open_reader(dir.path());
    assert_exhaustive(&reader, "reader over the compacted store");
}

/// A phrase repeating a term keeps every position: dropping the repeat would match "x y" alone.
#[test]
fn a_phrase_with_a_repeated_term_keeps_both_positions() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path());
    core.memorize_documents(vec![
        input("a", "x y x y"),
        input("b", "x y"),
        input("c", "y x y x"),
        input("d", "x x y y"),
    ])
    .unwrap();
    for compacted in [false, true] {
        if compacted {
            core.compact().unwrap();
        }
        for (phrase, want) in [
            ("\"x y x y\"", 1),
            ("\"x y x\"", 2),
            ("\"x y\"", 4),
            ("\"x x\"", 1),
        ] {
            let query = LexicalQuery::parse(phrase).unwrap();
            let options = LexicalOptions {
                k: 0,
                count: true,
                ..Default::default()
            };
            let count = core.search_lexical(&query, &options).unwrap().count;
            assert_eq!(count, Some(want), "{phrase} compacted={compacted}");
        }
    }
}
