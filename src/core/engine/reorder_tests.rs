// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! The slot reorder of an explicit compaction: every result (ids, scores, tie order and
//! counts) is the one the store gives in arrival order, after the reorder, after the
//! compaction, after a reopen, and after a crash at each step of the compaction.

use super::{ReorderOutcome, WarmCore};
use crate::core::documents::{
    DocumentInput, EncodeMode, IngestOptions, LexicalOptions, LexicalQuery, SearchOptions,
};
use crate::core::WarmConfig;
use anyhow::Result;
use serde_json::json;

/// More live documents than `REORDER_MIN_DOCUMENTS` after the deletes.
const DOCUMENTS: usize = 1600;

fn config(reorder: bool) -> WarmConfig {
    let mut config = WarmConfig {
        embedding_space: None,
        ..Default::default()
    };
    config.document_checkpoint.min_journal_bytes = 0;
    config.document_checkpoint.reorder = reorder;
    config
}

fn options(positions: bool) -> IngestOptions {
    IngestOptions {
        encode: EncodeMode::Lexical,
        positions,
        ..Default::default()
    }
}

/// Four topics interleaved by id, so arrival order scatters each topic's terms; every 50th
/// document repeats the previous one's text, so equal scores need the tie order.
fn text(i: usize) -> String {
    let source = if i % 50 == 49 { i - 1 } else { i };
    let topic = (source * 7919) % 4;
    let mut state = (source as u64)
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let mut next = move || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) as usize
    };
    let length = 8 + next() % 32;
    (0..length)
        .map(|_| {
            let draw = next();
            if draw % 3 == 0 {
                format!("c{}", (draw / 3) % 8)
            } else {
                format!("t{topic}w{}", (draw / 3) % 24 % (1 + (draw / 7) % 24))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn document(i: usize, version: &str, text: String) -> DocumentInput {
    serde_json::from_value(json!({"id": format!("d{i:05}"), "version": version, "text": text}))
        .unwrap()
}

/// Ingests, compacts without a reorder (a checkpoint in arrival order), then deletes and
/// replaces documents over that checkpoint.
fn build(core: &WarmCore, positions: bool) -> Result<()> {
    let options = options(positions);
    for start in (0..DOCUMENTS).step_by(200) {
        core.memorize_documents_with(
            (start..start + 200)
                .map(|i| document(i, "1", text(i)))
                .collect(),
            &options,
        )?;
    }
    core.compact_routine()?;
    for i in (0..DOCUMENTS).step_by(13) {
        core.delete_document(&format!("d{i:05}"))?;
    }
    core.memorize_documents_with(
        (5..DOCUMENTS)
            .step_by(11)
            .map(|i| document(i, "2", text(i + 3)))
            .collect(),
        &options,
    )?;
    Ok(())
}

fn answers(core: &WarmCore, positions: bool) -> Result<Vec<String>> {
    let mut queries = vec![
        "c0",
        "t1w0",
        "c1 t2w1",
        "t0w0 t3w2 c5",
        "+c0 +t1w0",
        "+c2 +c3 +t0w1",
        "absent",
    ];
    if positions {
        queries.extend(["\"c0 c1\"", "\"t2w0 t2w0\"", "+c4 \"t3w0 c2\""]);
    }
    let mut out = Vec::new();
    for query in queries {
        let parsed = LexicalQuery::parse(query)?;
        for (k, count) in [
            (10, false),
            (100, false),
            (0, true),
            (10, true),
            (1000, true),
        ] {
            let options = LexicalOptions {
                k,
                count,
                ..Default::default()
            };
            out.push(format!(
                "{query} {k} {count} {}",
                serde_json::to_string(&core.search_lexical(&parsed, &options)?)?
            ));
        }
    }
    let search: SearchOptions = serde_json::from_value(json!({"k": 200}))?;
    for query in ["c0 t1w0", "t3w1"] {
        out.push(serde_json::to_string(
            &core.search_documents(query, &search)?,
        )?);
    }
    Ok(out)
}

fn checkpoint(directory: &std::path::Path) -> Result<Vec<u8>> {
    let generation =
        crate::core::durable_file::arena_generation(&directory.join("vectors_data.bin"))?;
    Ok(std::fs::read(generation.join(super::image::FILE))?)
}

#[test]
fn reorder_keeps_every_result_through_compaction_and_reopen() -> Result<()> {
    // Compressed text keeps the checkpoint off the streamed merge: the slots are renumbered in
    // memory instead.
    for (positions, compress) in [(false, false), (true, false), (true, true)] {
        let mut config = config(true);
        config.document_checkpoint.compress_text = compress;
        let directory = tempfile::tempdir()?;
        let path = directory.path().display().to_string();
        let core = WarmCore::new(1024, Some(path.clone()), Some(config.clone()))?;
        build(&core, positions)?;
        let expected = answers(&core, positions)?;
        assert!(core.reorder_documents()?, "positions {positions}");
        assert_eq!(
            answers(&core, positions)?,
            expected,
            "reordered, positions {positions}"
        );
        core.compact()?;
        assert_eq!(
            answers(&core, positions)?,
            expected,
            "compacted, positions {positions}"
        );
        drop(core);
        let core = WarmCore::new(1024, Some(path), Some(config.clone()))?;
        assert_eq!(
            answers(&core, positions)?,
            expected,
            "reopened, positions {positions}"
        );
        // A reorder of the reordered store, served from its checkpoint, changes nothing.
        core.compact()?;
        assert_eq!(
            answers(&core, positions)?,
            expected,
            "again, positions {positions}"
        );
    }
    Ok(())
}

/// The reorder is what makes the checkpoint differ, and the switch turns it off: with it off,
/// `compact` writes the checkpoint the routine compaction writes.
#[test]
fn reorder_switch_decides_the_checkpoint_and_never_the_results() -> Result<()> {
    for positions in [false, true] {
        let mut written = Vec::new();
        let mut results = Vec::new();
        for (reorder, routine) in [(true, false), (false, false), (true, true)] {
            let directory = tempfile::tempdir()?;
            let core = WarmCore::new(
                1024,
                Some(directory.path().display().to_string()),
                Some(config(reorder)),
            )?;
            build(&core, positions)?;
            if routine {
                core.compact_routine()?;
            } else {
                core.compact()?;
            }
            results.push(answers(&core, positions)?);
            drop(core);
            // Past the magic, the file CRC and the generation name's CRC.
            written.push(checkpoint(directory.path())?[24..].to_vec());
        }
        assert_eq!(results[0], results[1], "positions {positions}");
        assert_eq!(results[1], results[2], "positions {positions}");
        assert_eq!(written[1], written[2], "off is the routine compaction");
        assert!(
            written[0].len() < written[1].len(),
            "the reordered checkpoint is smaller, positions {positions}: {} vs {}",
            written[0].len(),
            written[1].len()
        );
    }
    Ok(())
}

fn header_word(directory: &std::path::Path, index: usize) -> Result<u64> {
    let bytes = checkpoint(directory)?;
    Ok(u64::from_le_bytes(
        bytes[8 + 8 * index..16 + 8 * index].try_into()?,
    ))
}

/// `compact` reorders a store that was never reordered, then only once a quarter of its slots
/// came after the last reorder; header word 5 records the reordered slots and word 8 all.
#[test]
fn compaction_reorders_again_only_after_a_quarter_of_the_slots_are_new() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().display().to_string();
    let core = WarmCore::new(1024, Some(path), Some(config(true)))?;
    let options = options(true);
    let add = |range: std::ops::Range<usize>| {
        core.memorize_documents_with(range.map(|i| document(i, "1", text(i))).collect(), &options)
    };
    add(0..1200)?;
    core.compact()?;
    let (ordered, slots) = (
        header_word(directory.path(), 5)?,
        header_word(directory.path(), 8)?,
    );
    assert_eq!((ordered, slots), (1200, 1200));
    add(1200..1500)?;
    core.compact()?;
    assert_eq!(
        (
            header_word(directory.path(), 5)?,
            header_word(directory.path(), 8)?
        ),
        (1200, 1500),
        "300 of 1500 slots are new: not due"
    );
    add(1500..1600)?;
    // The checkpoint's lists and the live store's merge in the new order.
    let expected = answers(&core, true)?;
    core.compact()?;
    assert_eq!(core.last_reorder().outcome, ReorderOutcome::Reordered);
    assert_eq!(
        (
            header_word(directory.path(), 5)?,
            header_word(directory.path(), 8)?
        ),
        (1600, 1600),
        "400 of 1600 slots are new: due"
    );
    assert_eq!(answers(&core, true)?, expected);
    drop(core);
    let core = WarmCore::new(
        1024,
        Some(directory.path().display().to_string()),
        Some(config(true)),
    )?;
    assert_eq!(answers(&core, true)?, expected, "reopened");
    Ok(())
}

/// Over `reorder_memory_budget`, `compact` writes the checkpoint in arrival order and reports
/// the estimate; `reorder_documents` reorders anyway.
#[test]
fn reorder_over_budget_is_skipped_and_reported_unless_forced() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let mut config = config(true);
    config.document_checkpoint.reorder_memory_budget = 1;
    let core = WarmCore::new(
        1024,
        Some(directory.path().display().to_string()),
        Some(config),
    )?;
    build(&core, true)?;
    let expected = answers(&core, true)?;
    core.compact()?;
    let report = core.last_reorder();
    assert_eq!(report.outcome, ReorderOutcome::OverBudget);
    assert!(
        report.estimate_bytes > 1 && report.budget_bytes == 1,
        "{report:?}"
    );
    assert_eq!(header_word(directory.path(), 5)?, 0);
    assert!(core.reorder_documents()?);
    assert_eq!(core.last_reorder().outcome, ReorderOutcome::Reordered);
    assert_eq!(
        header_word(directory.path(), 5)?,
        header_word(directory.path(), 8)?
    );
    assert_eq!(answers(&core, true)?, expected);
    Ok(())
}

#[test]
fn reorder_crash_child() -> Result<()> {
    let Ok(root) = crate::compatibility::var("WARM_TEST_REORDER_ROOT") else {
        return Ok(());
    };
    let core = WarmCore::new(1024, Some(root), Some(config(true)))?;
    core.compact()
}

/// A crash at each step of a reordering compaction reopens to the old checkpoint or the new
/// one, and both give every result the store gave before.
#[test]
fn reordering_compaction_crash_reopens_to_old_or_new() -> Result<()> {
    for positions in [false, true] {
        for phase in [
            "reordered",
            "fold-postings",
            "fold-spooled",
            "fold-written",
            "caught-up",
            "staged",
            "published",
            "mapped",
            "released",
            "finished",
        ] {
            let directory = tempfile::tempdir()?;
            let path = directory.path().display().to_string();
            let expected = {
                let core = WarmCore::new(1024, Some(path.clone()), Some(config(true)))?;
                build(&core, positions)?;
                answers(&core, positions)?
            };
            let output = std::process::Command::new(std::env::current_exe()?)
                .args([
                    "--exact",
                    "core::engine::reorder_tests::reorder_crash_child",
                ])
                .env("WARM_TEST_REORDER_ROOT", directory.path())
                .env("WARM_TEST_COMPACTION_PHASE", phase)
                .output()?;
            assert_eq!(output.status.code(), Some(73), "{phase}: {output:?}");
            let core = WarmCore::new(1024, Some(path), Some(config(true)))?;
            assert_eq!(
                answers(&core, positions)?,
                expected,
                "{phase}, positions {positions}"
            );
        }
    }
    Ok(())
}
