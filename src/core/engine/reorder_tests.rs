// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! The slot reorder of an explicit compaction: every result (ids, scores, tie order and
//! counts) is the one the store gives in arrival order, after the reorder, after the
//! compaction, after a reopen, and after a crash at each step of the compaction.

use super::background_compaction::JobState;
use super::compaction::ReorderRequest;
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
    core.compact_without_reorder()?;
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
                core.compact_without_reorder()?;
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

/// Like [`document`], cut into chunks of 8 words, so most documents hold several chunks.
fn chunked(i: usize, version: &str, text: String) -> DocumentInput {
    serde_json::from_value(json!({
        "id": format!("d{i:05}"), "version": version, "text": text, "chunkWords": 8,
    }))
    .unwrap()
}

/// A longer text than [`text`]: several of its texts joined.
fn long_text(i: usize) -> String {
    (0..4)
        .map(|k| text(i * 4 + k))
        .collect::<Vec<_>>()
        .join(" ")
}

fn ingest(core: &WarmCore, range: std::ops::Range<usize>, version: &str) -> Result<()> {
    let options = options(true);
    let ids: Vec<usize> = range.collect();
    for batch in ids.chunks(200) {
        core.memorize_documents_with(
            batch
                .iter()
                .map(|&i| chunked(i, version, text(i)))
                .collect(),
            &options,
        )?;
    }
    Ok(())
}

/// Every chunk of every document under `d00000..d{limit}`, read back by id and index.
fn chunks(core: &WarmCore, limit: usize) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for i in 0..limit {
        let id = format!("d{i:05}");
        let Some(record) = core.get_document(&id)? else {
            continue;
        };
        for index in 0..record.chunk_count {
            let chunk = core
                .read_document_chunk(&id, index)?
                .ok_or_else(|| anyhow::anyhow!("{id} chunk {index} missing"))?;
            out.push(format!(
                "{id} {} {} {} {} {:?}",
                record.version, chunk.index, chunk.start_byte, chunk.end_byte, chunk.text
            ));
        }
    }
    Ok(out)
}

fn state(core: &WarmCore, limit: usize) -> Result<(Vec<String>, Vec<String>)> {
    Ok((answers(core, true)?, chunks(core, limit)?))
}

/// Multi-chunk documents, some written into slots that deletes freed, keep every hit (with its
/// chunk index) and every chunk through the reorder of both branches: the streamed merge and,
/// with compressed text, the renumbering in memory.
#[test]
fn reorder_keeps_multi_chunk_documents_and_rewrites_into_freed_slots() -> Result<()> {
    for compress in [false, true] {
        let mut config = config(true);
        config.document_checkpoint.compress_text = compress;
        let directory = tempfile::tempdir()?;
        let path = directory.path().display().to_string();
        let core = WarmCore::new(1024, Some(path.clone()), Some(config.clone()))?;
        ingest(&core, 0..DOCUMENTS, "1")?;
        for i in (0..DOCUMENTS).step_by(9) {
            core.delete_document(&format!("d{i:05}"))?;
        }
        // New documents and longer versions take the freed slots first, lowest first.
        core.memorize_documents_with(
            (0..6)
                .map(|k| chunked(DOCUMENTS + k, "1", long_text(k)))
                .chain((4..60).step_by(11).map(|i| chunked(i, "2", long_text(i))))
                .collect(),
            &options(true),
        )?;
        let expected = state(&core, DOCUMENTS + 6)?;
        assert!(
            expected.1.len() > 2 * DOCUMENTS,
            "documents hold several chunks"
        );
        assert!(!core.documents.read().has_base());
        assert!(core.reorder_documents()?, "compress {compress}");
        assert_eq!(
            state(&core, DOCUMENTS + 6)?,
            expected,
            "compress {compress}"
        );
        drop(core);
        let core = WarmCore::new(1024, Some(path), Some(config))?;
        assert_eq!(
            state(&core, DOCUMENTS + 6)?,
            expected,
            "reopened, compress {compress}"
        );
    }
    Ok(())
}

/// A checkpoint with free slots (deleted documents compacted away) that the store is served
/// from, intact, with documents written, deleted and replaced since.
fn served_base_with_free_slots(core: &WarmCore) -> Result<()> {
    ingest(core, 0..DOCUMENTS, "1")?;
    core.compact_without_reorder()?;
    for i in (0..DOCUMENTS).step_by(13) {
        core.delete_document(&format!("d{i:05}"))?;
    }
    core.compact_without_reorder()?;
    Ok(())
}

/// Writes after the checkpoint, touching only documents written after it, so the checkpoint
/// stays intact.
fn live_writes(core: &WarmCore) -> Result<()> {
    ingest(core, DOCUMENTS..DOCUMENTS + 400, "1")?;
    for i in (DOCUMENTS..DOCUMENTS + 400).step_by(7) {
        core.delete_document(&format!("d{i:05}"))?;
    }
    core.memorize_documents_with(
        (DOCUMENTS + 1..DOCUMENTS + 400)
            .step_by(11)
            .map(|i| chunked(i, "2", long_text(i)))
            .collect(),
        &options(true),
    )?;
    Ok(())
}

const LIVE_LIMIT: usize = DOCUMENTS + 400;

fn assert_intact_base(core: &WarmCore) {
    let store = core.documents.read();
    assert!(store.has_base() && store.can_merge_checkpoint());
}

/// The streamed merge in bisection order over a served checkpoint that has free slots, plus
/// documents written, deleted and replaced since it, keeps every result and chunk.
#[test]
fn reorder_merges_an_intact_checkpoint_with_free_slots_and_live_changes() -> Result<()> {
    // From a twin: reading this store before the compaction would rebuild it in memory and
    // take the checkpoint out of the merge.
    let reference = tempfile::tempdir()?;
    let expected = {
        let core = WarmCore::new(
            1024,
            Some(reference.path().display().to_string()),
            Some(config(false)),
        )?;
        served_base_with_free_slots(&core)?;
        live_writes(&core)?;
        state(&core, LIVE_LIMIT)?
    };
    let directory = tempfile::tempdir()?;
    let path = directory.path().display().to_string();
    let core = WarmCore::new(1024, Some(path.clone()), Some(config(true)))?;
    served_base_with_free_slots(&core)?;
    assert_intact_base(&core);
    let (slots, chunk_count) = (
        header_word(directory.path(), 8)?,
        header_word(directory.path(), 9)?,
    );
    assert!(slots > chunk_count, "free slots: {slots} > {chunk_count}");
    live_writes(&core)?;
    assert_intact_base(&core);
    core.compact()?;
    assert_eq!(core.last_reorder().outcome, ReorderOutcome::Reordered);
    let (ordered, slots, chunk_count) = (
        header_word(directory.path(), 5)?,
        header_word(directory.path(), 8)?,
        header_word(directory.path(), 9)?,
    );
    assert_eq!((ordered, slots), (chunk_count, chunk_count), "no free slot");
    // Served from the reordered checkpoint.
    assert!(core.documents.read().has_base());
    assert_eq!(state(&core, LIVE_LIMIT)?, expected);
    drop(core);
    let core = WarmCore::new(1024, Some(path), Some(config(true)))?;
    assert!(core.documents.read().has_base());
    assert_eq!(state(&core, LIVE_LIMIT)?, expected, "reopened");
    // Rebuilt in memory from the reordered checkpoint's slot table.
    core.hydrate_documents()?;
    assert!(!core.documents.read().has_base());
    assert_eq!(state(&core, LIVE_LIMIT)?, expected, "hydrated");
    Ok(())
}

/// Writes that land while a reordering compaction writes its generation (the published
/// checkpoint is in the new order, the store keeps serving the old one, and the writes are
/// replayed by id over the new one) give the results of a store that never reordered.
#[test]
fn writes_during_a_reordering_compaction_survive_it_and_reopen() -> Result<()> {
    let writes = |core: &WarmCore| -> Result<()> {
        live_writes(core)?;
        // Over the checkpoint itself: a delete and a longer version of its documents.
        core.delete_document("d00003")?;
        core.memorize_documents_with(vec![chunked(10, "2", long_text(10))], &options(true))?;
        Ok(())
    };
    let reference = tempfile::tempdir()?;
    let expected = {
        let core = WarmCore::new(
            1024,
            Some(reference.path().display().to_string()),
            Some(config(false)),
        )?;
        served_base_with_free_slots(&core)?;
        writes(&core)?;
        state(&core, LIVE_LIMIT)?
    };
    let directory = tempfile::tempdir()?;
    let path = directory.path().display().to_string();
    let core = WarmCore::new(1024, Some(path.clone()), Some(config(true)))?;
    served_base_with_free_slots(&core)?;
    let job = JobState::foreground().reordering(ReorderRequest::WithinBudget);
    core.compact_impl(|| writes(&core).unwrap(), || {}, false, &job)?;
    assert!(job.tail_frames.load(std::sync::atomic::Ordering::Relaxed) > 0);
    assert_eq!(core.last_reorder().outcome, ReorderOutcome::Reordered);
    assert_eq!(
        header_word(directory.path(), 5)?,
        header_word(directory.path(), 8)?
    );
    assert_eq!(state(&core, LIVE_LIMIT)?, expected, "served");
    drop(core);
    let core = WarmCore::new(1024, Some(path), Some(config(true)))?;
    assert_eq!(state(&core, LIVE_LIMIT)?, expected, "reopened");
    core.compact_without_reorder()?;
    assert_eq!(state(&core, LIVE_LIMIT)?, expected, "folded");
    Ok(())
}

#[test]
fn reorder_merge_crash_child() -> Result<()> {
    let Ok(root) = crate::compatibility::var("WARM_TEST_REORDER_MERGE_ROOT") else {
        return Ok(());
    };
    let core = WarmCore::new(1024, Some(root), Some(config(true)))?;
    live_writes(&core)?;
    assert_intact_base(&core);
    core.compact()
}

/// A crash inside the ordered merge of a served checkpoint and the documents written since
/// reopens to the old checkpoint or the new one, with every result and chunk.
#[test]
fn ordered_merge_over_a_checkpoint_crash_reopens_to_old_or_new() -> Result<()> {
    let reference = tempfile::tempdir()?;
    let expected = {
        let core = WarmCore::new(
            1024,
            Some(reference.path().display().to_string()),
            Some(config(false)),
        )?;
        served_base_with_free_slots(&core)?;
        live_writes(&core)?;
        state(&core, LIVE_LIMIT)?
    };
    for phase in [
        "reordered",
        "fold-postings",
        "fold-spooled",
        "fold-written",
        "published",
    ] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().display().to_string();
        {
            let core = WarmCore::new(1024, Some(path.clone()), Some(config(true)))?;
            served_base_with_free_slots(&core)?;
        }
        let output = std::process::Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "core::engine::reorder_tests::reorder_merge_crash_child",
            ])
            .env("WARM_TEST_REORDER_MERGE_ROOT", directory.path())
            .env("WARM_TEST_COMPACTION_PHASE", phase)
            .output()?;
        assert_eq!(output.status.code(), Some(73), "{phase}: {output:?}");
        let core = WarmCore::new(1024, Some(path), Some(config(true)))?;
        assert_eq!(state(&core, LIVE_LIMIT)?, expected, "{phase}");
    }
    Ok(())
}

/// A store that writes no checkpoint keeps its order: no reorder runs (nothing is estimated
/// and the slots are not renumbered in memory), both calls report `NoCheckpoint`, and
/// `reorder_documents` returns false.
#[test]
fn stores_that_write_no_checkpoint_are_not_reordered() -> Result<()> {
    for trigger in ["checkpoint disabled", "relation", "schema 3"] {
        let mut config = config(true);
        if trigger == "checkpoint disabled" {
            config.document_checkpoint.enabled = false;
        }
        let directory = tempfile::tempdir()?;
        let mut core = WarmCore::new(
            1024,
            Some(directory.path().display().to_string()),
            Some(config.clone()),
        )?;
        if trigger == "schema 3" {
            core.schema = crate::core::format::CHECKPOINT_SCHEMA_VERSION - 1;
        }
        ingest(&core, 0..DOCUMENTS, "1")?;
        if trigger == "relation" {
            core.add_relation(&crate::core::types::Relation {
                source_id: "d00001".into(),
                relation_type: "cites".into(),
                target_id: "d00002".into(),
                properties: None,
                valid_from: 0.0,
                valid_to: 0.0,
            })?;
        }
        let expected = state(&core, DOCUMENTS)?;
        let skipped = super::ReorderReport {
            outcome: ReorderOutcome::NoCheckpoint,
            estimate_bytes: 0,
            budget_bytes: config.document_checkpoint.reorder_memory_budget,
        };
        core.compact()?;
        assert_eq!(core.last_reorder(), skipped, "{trigger}");
        assert!(core.documents.read().reorder_due(), "{trigger}: renumbered");
        assert_eq!(state(&core, DOCUMENTS)?, expected, "{trigger}");
        assert!(!core.reorder_documents()?, "{trigger}");
        assert_eq!(core.last_reorder(), skipped, "{trigger}");
        assert!(core.documents.read().reorder_due(), "{trigger}: renumbered");
        assert_eq!(state(&core, DOCUMENTS)?, expected, "{trigger}");
        assert!(
            checkpoint(directory.path()).is_err(),
            "{trigger}: checkpoint"
        );
    }
    Ok(())
}

/// The report is recorded when the generation is published: a reordering compaction that is
/// cancelled before then leaves the previous report.
#[test]
fn reorder_report_waits_for_publication() -> Result<()> {
    for compress in [false, true] {
        let mut config = config(true);
        config.document_checkpoint.compress_text = compress;
        let directory = tempfile::tempdir()?;
        let core = WarmCore::new(
            1024,
            Some(directory.path().display().to_string()),
            Some(config),
        )?;
        ingest(&core, 0..DOCUMENTS, "1")?;
        let expected = state(&core, DOCUMENTS)?;
        let job = JobState::foreground().reordering(ReorderRequest::WithinBudget);
        assert!(core
            .compact_impl(|| {}, || job.request_cancel(), false, &job)
            .is_err());
        assert_eq!(
            core.last_reorder().outcome,
            ReorderOutcome::NotRequested,
            "compress {compress}"
        );
        assert!(checkpoint(directory.path()).is_err());
        assert_eq!(state(&core, DOCUMENTS)?, expected, "compress {compress}");
        core.compact()?;
        // An in-place renumbering that was not published is kept and written now.
        let outcome = core.last_reorder().outcome;
        if compress {
            assert_eq!(outcome, ReorderOutcome::NotDue);
        } else {
            assert_eq!(outcome, ReorderOutcome::Reordered);
        }
        assert_eq!(
            header_word(directory.path(), 5)?,
            header_word(directory.path(), 8)?
        );
        assert_eq!(state(&core, DOCUMENTS)?, expected, "compress {compress}");
    }
    Ok(())
}
