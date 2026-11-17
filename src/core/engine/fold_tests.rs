// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Folding the live document index into the checkpoint during ingest: the folded store is the
//! store a single pass writes, and a crash at any point of a fold reopens to the batches that
//! were committed.

use super::WarmCore;
use crate::core::documents::{DocumentInput, EncodeMode, IngestOptions, SearchOptions};
use crate::core::WarmConfig;
use anyhow::{ensure, Result};
use serde_json::json;

const WORDS: [&str; 12] = [
    "amber", "binary", "cedar", "delta", "ember", "fjord", "garnet", "harbor", "indigo", "juniper",
    "kernel", "lantern",
];

fn config(budget: usize) -> WarmConfig {
    let mut config = WarmConfig::default();
    config.document_checkpoint.min_journal_bytes = 0;
    config.document_checkpoint.index_memory_budget = budget;
    // A fold after every batch that crosses the budget: the crash tests count folds by batch.
    config.document_checkpoint.index_fold_fraction = 0.0;
    config
}

fn options() -> IngestOptions {
    IngestOptions {
        encode: EncodeMode::Lexical,
        ..Default::default()
    }
}

fn document(i: usize, retain: bool) -> DocumentInput {
    let text: Vec<String> = (0..4 + i % 9)
        .map(|j| format!("{}{}", WORDS[(i * 5 + j * 3) % 12], (i + j) % 7))
        .chain(["common".to_owned(), WORDS[i % 12].to_owned()])
        .collect();
    serde_json::from_value(json!({
        "id": format!("d{i:04}"), "text": text.join(" "), "storeText": retain,
        "metadata": {"bucket": i % 3},
    }))
    .unwrap()
}

fn ingest(core: &WarmCore, documents: usize, batch: usize, retain: bool) -> Result<()> {
    for start in (0..documents).step_by(batch) {
        let end = (start + batch).min(documents);
        core.memorize_documents_with(
            (start..end).map(|i| document(i, retain)).collect(),
            &options(),
        )?;
    }
    Ok(())
}

fn answers(core: &WarmCore) -> Result<Vec<String>> {
    let search: SearchOptions = serde_json::from_value(
        json!({"k": 200, "candidateLimit": 200, "lexicalWeight": 1, "semanticWeight": 0}),
    )?;
    [
        "common",
        "amber1 binary2",
        "cedar delta5 juniper",
        "lantern0",
        "absent",
    ]
    .iter()
    .map(|query| {
        Ok(serde_json::to_string(
            &core.search_documents(query, &search)?,
        )?)
    })
    .collect()
}

fn checkpoint(directory: &std::path::Path) -> Vec<u8> {
    fn find(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                find(&path, found);
            } else if path
                .file_name()
                .is_some_and(|name| name == "documents.ckpt")
            {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    find(directory, &mut found);
    assert_eq!(found.len(), 1, "one live checkpoint: {found:?}");
    std::fs::read(&found[0]).unwrap()
}

/// The checkpoint bytes that do not name the generation: its CRC words (8..24) differ by name.
fn body(bytes: &[u8]) -> &[u8] {
    &bytes[24..]
}

#[test]
fn folded_ingest_writes_the_checkpoint_of_a_single_pass() -> Result<()> {
    for retain in [false, true] {
        let whole = tempfile::tempdir()?;
        let folded = tempfile::tempdir()?;
        let expected = {
            let core = WarmCore::new(
                256,
                Some(whole.path().display().to_string()),
                Some(config(0)),
            )?;
            ingest(&core, 70, 7, retain)?;
            assert!(!core.serves_from_checkpoint(), "a zero budget never folds");
            answers(&core)?
        };
        {
            let core = WarmCore::new(
                256,
                Some(folded.path().display().to_string()),
                Some(config(1)),
            )?;
            ingest(&core, 70, 7, retain)?;
            assert!(core.serves_from_checkpoint(), "retain={retain}: folded");
            assert_eq!(answers(&core)?, expected, "retain={retain}: live answers");
        }
        assert_eq!(
            body(&checkpoint(whole.path())),
            body(&checkpoint(folded.path())),
            "retain={retain}: the checkpoints differ"
        );
        let bytes = checkpoint(folded.path());
        assert_eq!(&bytes[..8], b"HMSCKP\x00\x04", "retain={retain}: version 4");
        let reopened = WarmCore::new(
            256,
            Some(folded.path().display().to_string()),
            Some(config(1)),
        )?;
        assert!(reopened.serves_from_checkpoint());
        assert_eq!(
            answers(&reopened)?,
            expected,
            "retain={retain}: reopened answers"
        );
    }
    Ok(())
}

#[test]
fn documents_replaced_or_deleted_after_a_fold_answer_as_without_folding() -> Result<()> {
    let run = |budget: usize| -> Result<(Vec<String>, Vec<String>)> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().display().to_string();
        let live = {
            let core = WarmCore::new(256, Some(path.clone()), Some(config(budget)))?;
            ingest(&core, 40, 5, false)?;
            let mut changed = document(7, false);
            changed.text = "replaced lantern0 common".into();
            core.memorize_document(changed)?;
            ensure!(core.delete_document("d0012")?);
            ingest(&core, 55, 5, false)?;
            answers(&core)?
        };
        let reopened = WarmCore::new(256, Some(path), Some(config(budget)))?;
        Ok((live, answers(&reopened)?))
    };
    assert_eq!(run(1)?, run(0)?);
    Ok(())
}

/// Child of [`a_crash_during_a_fold_reopens_to_the_committed_batches`].
#[test]
fn fold_crash_child() -> Result<()> {
    let Ok(root) = crate::compatibility::var("WARM_TEST_FOLD_ROOT") else {
        return Ok(());
    };
    let core = WarmCore::new(256, Some(root), Some(config(1)))?;
    ingest(&core, 60, 5, false)
}

#[test]
fn a_crash_during_a_fold_reopens_to_the_committed_batches() -> Result<()> {
    let mut crashed = 0;
    for phase in [
        "fold-postings",
        "fold-spooled",
        "fold-written",
        "staged",
        "caught-up",
        "published",
        "mapped",
        "released",
    ] {
        for skip in [0usize, 1, 4] {
            let directory = tempfile::tempdir()?;
            let output = std::process::Command::new(std::env::current_exe()?)
                .args(["--exact", "core::engine::fold_tests::fold_crash_child"])
                .env("WARM_TEST_FOLD_ROOT", directory.path())
                .env("WARM_TEST_COMPACTION_PHASE", phase)
                .env("WARM_TEST_COMPACTION_SKIP", skip.to_string())
                .output()?;
            if output.status.code() != Some(73) {
                continue;
            }
            crashed += 1;
            // Each fold follows a committed batch of five, so the (skip+1)th hit of a phase
            // is the (skip+1)th fold, after batch skip+1.
            let committed = (skip + 1) * 5;
            let core = WarmCore::new(
                256,
                Some(directory.path().display().to_string()),
                Some(config(1)),
            )?;
            let reference = tempfile::tempdir()?;
            let expected = {
                let whole = WarmCore::new(
                    256,
                    Some(reference.path().display().to_string()),
                    Some(config(0)),
                )?;
                ingest(&whole, committed, 5, false)?;
                answers(&whole)?
            };
            assert_eq!(answers(&core)?, expected, "{phase} after fold {}", skip + 1);
            // The reopened store takes up where the crash left it.
            ingest(&core, committed + 10, 5, false).ok();
        }
    }
    assert!(crashed >= 20, "only {crashed} crash points were reached");
    Ok(())
}
#[test]
fn geometric_folds_answer_as_a_single_pass() -> Result<()> {
    let run = |budget: usize, fraction: f64| -> Result<Vec<String>> {
        let directory = tempfile::tempdir()?;
        let mut config = config(budget);
        config.document_checkpoint.index_fold_fraction = fraction;
        let core = WarmCore::new(
            256,
            Some(directory.path().display().to_string()),
            Some(config),
        )?;
        ingest(&core, 60, 5, false)?;
        answers(&core)
    };
    let whole = run(0, 0.0)?;
    for fraction in [0.5, 2.0, 20.0] {
        assert_eq!(run(1, fraction)?, whole, "fraction {fraction}");
    }
    Ok(())
}

fn with_text(budget: usize, compress: bool) -> WarmConfig {
    let mut config = config(budget);
    config.document_checkpoint.compress_text = compress;
    config
}

fn ingest_range(core: &WarmCore, from: usize, to: usize, batch: usize) -> Result<()> {
    for start in (from..to).step_by(batch) {
        core.memorize_documents_with(
            (start..(start + batch).min(to))
                .map(|i| document(i, true))
                .collect(),
            &options(),
        )?;
    }
    Ok(())
}

/// The feature word of the live checkpoint (header word 3).
fn features(directory: &std::path::Path) -> u64 {
    u64::from_le_bytes(checkpoint(directory)[32..40].try_into().unwrap())
}

fn stored_texts(core: &WarmCore, documents: usize) -> Result<Vec<Option<String>>> {
    let search: SearchOptions = serde_json::from_value(
        json!({"k": 500, "candidateLimit": 500, "lexicalWeight": 1, "semanticWeight": 0}),
    )?;
    let mut texts = vec![None; documents];
    for hit in core.search_documents("common", &search)? {
        let i: usize = hit.document_id[1..].parse()?;
        texts[i] = hit.text.clone();
    }
    Ok(texts)
}

/// A store with `compress_text` on, ingested with a fold budget that would fold it, ranks and
/// returns text as the unfolded store and as an uncompressed one, and holds compressed text.
#[test]
fn compressed_text_stores_fold_to_the_answers_of_an_unfolded_store() -> Result<()> {
    let open = |dir: &tempfile::TempDir, budget, compress| {
        WarmCore::new(
            256,
            Some(dir.path().display().to_string()),
            Some(with_text(budget, compress)),
        )
    };
    let (plain, unfolded, folded) = (
        tempfile::tempdir()?,
        tempfile::tempdir()?,
        tempfile::tempdir()?,
    );
    let expected = {
        let core = open(&plain, 0, false)?;
        ingest_range(&core, 0, 70, 7)?;
        (answers(&core)?, stored_texts(&core, 70)?)
    };
    assert!(expected.1.iter().all(Option::is_some));
    {
        let core = open(&unfolded, 0, true)?;
        ingest_range(&core, 0, 70, 7)?;
        assert_eq!((answers(&core)?, stored_texts(&core, 70)?), expected);
    }
    {
        let core = open(&folded, 1, true)?;
        ingest_range(&core, 0, 70, 7)?;
        assert_eq!((answers(&core)?, stored_texts(&core, 70)?), expected);
    }
    for dir in [&unfolded, &folded] {
        assert_eq!(features(dir.path()), 1, "text blocks");
        let core = open(dir, 1, true)?;
        assert!(core.serves_from_checkpoint());
        assert_eq!((answers(&core)?, stored_texts(&core, 70)?), expected);
    }
    assert_eq!(features(plain.path()), 0);
    Ok(())
}

/// A store that toggles `compress_text` between checkpoints: each compaction writes what the
/// setting says, and every state answers as a store that never toggled.
#[test]
fn toggling_compress_text_between_folds_keeps_answers_and_text() -> Result<()> {
    let (toggled, reference) = (tempfile::tempdir()?, tempfile::tempdir()?);
    let open = |dir: &tempfile::TempDir, budget, compress| {
        WarmCore::new(
            256,
            Some(dir.path().display().to_string()),
            Some(with_text(budget, compress)),
        )
    };
    {
        let core = open(&reference, 0, false)?;
        ingest_range(&core, 0, 90, 5)?;
    }
    let expected = {
        let core = open(&reference, 0, false)?;
        (answers(&core)?, stored_texts(&core, 90)?)
    };
    // on, off, on: the second leg folds over a compressed-text base and the third over inline.
    for (leg, compress, expect_features) in [(0, true, 1), (1, false, 0), (2, true, 1)] {
        let core = open(&toggled, 1, compress)?;
        ingest_range(&core, leg * 30, leg * 30 + 30, 5)?;
        drop(core);
        assert_eq!(features(toggled.path()), expect_features, "leg {leg}");
        let core = open(&toggled, 1, compress)?;
        let seen = (leg + 1) * 30;
        assert_eq!(stored_texts(&core, 90)?[..seen].len(), seen);
        assert!(stored_texts(&core, 90)?[..seen].iter().all(Option::is_some));
    }
    let core = open(&toggled, 1, true)?;
    assert_eq!((answers(&core)?, stored_texts(&core, 90)?), expected);
    Ok(())
}

/// A store that keeps word positions folds to the version 4 checkpoint a single pass writes,
/// positions and skip tables included, and answers phrases the same after reopening.
#[test]
fn positional_stores_fold_to_the_checkpoint_of_a_single_pass() -> Result<()> {
    let positional = IngestOptions {
        positions: true,
        ..options()
    };
    let ingest_positional = |core: &WarmCore| -> Result<()> {
        for start in (0..70).step_by(7) {
            core.memorize_documents_with(
                (start..start + 7).map(|i| document(i, false)).collect(),
                &positional,
            )?;
        }
        Ok(())
    };
    let phrases = |core: &WarmCore| -> Result<Vec<String>> {
        [
            "\"common amber0\"",
            "\"binary2 common\"",
            "\"amber1 binary2\"",
        ]
        .iter()
        .map(|q| {
            let hits = core.search_lexical(
                &crate::core::documents::LexicalQuery::parse(q)?,
                &crate::core::documents::LexicalOptions {
                    k: 50,
                    count: true,
                    ..Default::default()
                },
            )?;
            Ok(format!("{:?}", hits))
        })
        .collect()
    };
    let whole = tempfile::tempdir()?;
    let folded = tempfile::tempdir()?;
    let expected = {
        let core = WarmCore::new(
            256,
            Some(whole.path().display().to_string()),
            Some(config(0)),
        )?;
        ingest_positional(&core)?;
        (answers(&core)?, phrases(&core)?)
    };
    {
        let core = WarmCore::new(
            256,
            Some(folded.path().display().to_string()),
            Some(config(1)),
        )?;
        ingest_positional(&core)?;
        assert!(core.serves_from_checkpoint(), "folded");
    }
    let (whole_bytes, folded_bytes) = (checkpoint(whole.path()), checkpoint(folded.path()));
    assert_eq!(&folded_bytes[..8], b"HMSCKP\x00\x04");
    assert_eq!(folded_bytes[40] & 1, 1, "positional flag");
    assert_eq!(body(&whole_bytes), body(&folded_bytes));
    let reopened = WarmCore::new(
        256,
        Some(folded.path().display().to_string()),
        Some(config(1)),
    )?;
    assert!(reopened.serves_from_checkpoint());
    assert_eq!((answers(&reopened)?, phrases(&reopened)?), expected);
    Ok(())
}
