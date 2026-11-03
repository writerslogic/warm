// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::core::engine::compaction::tests::{
    assert_current, config, configurations, document, mutate, seed,
};
use std::sync::mpsc;

fn open(path: &std::path::Path, config: &crate::core::WarmConfig) -> Result<Arc<WarmCore>> {
    Ok(Arc::new(WarmCore::new(
        1024,
        Some(path.display().to_string()),
        Some(config.clone()),
    )?))
}

/// Generation directories under the arena: after any finished job, only the live one.
fn generations(path: &std::path::Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(path.join("vectors_data.bin"))? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.starts_with("gen-") {
            names.push(name);
        }
    }
    Ok(names)
}

/// Garbage to reclaim that leaves the live state unchanged: vectors overwritten several times,
/// then deleted.
fn churn(core: &WarmCore) -> Result<()> {
    for round in 0..4 {
        for i in 0..32 {
            core.memorize(
                format!("v{i}"),
                crate::EntangledHVec::new_deterministic(1024, round * 100 + i),
            )?;
        }
    }
    for i in 0..32 {
        core.delete(&format!("v{i}"))?;
    }
    Ok(())
}

fn none() -> Hook {
    Box::new(|| {})
}

#[test]
fn background_compaction_reports_progress_reclaims_and_survives_reopen() -> Result<()> {
    for config in configurations() {
        let dir = tempfile::tempdir()?;
        let core = open(dir.path(), &config)?;
        seed(&core)?;
        churn(&core)?;
        mutate(&core)?;
        let handle = core.start_compaction(CompactionOptions::default())?;
        let report = handle.wait();
        assert_eq!(report.status, CompactionStatus::Completed, "{report:?}");
        assert!(report.published);
        assert_eq!(report.progress.stage, CompactionStage::Finished);
        assert_eq!(handle.progress().stage, CompactionStage::Finished);
        assert!(handle.is_finished());
        assert_eq!(report.progress.frames_written, report.progress.frames_total);
        assert!(report.progress.frames_total > 0);
        assert!(report.disk_bytes_before > 0 && report.disk_bytes_after > 0);
        assert!(report.progress.bytes_written > 0);
        assert_eq!(
            report.reclaimed_bytes,
            report
                .disk_bytes_before
                .saturating_sub(report.disk_bytes_after)
        );
        assert_eq!(
            report.write_amplification,
            (report.reclaimed_bytes > 0)
                .then(|| report.progress.bytes_written as f64 / report.reclaimed_bytes as f64)
        );
        // A second wait returns the same report.
        assert_eq!(handle.wait(), report);
        assert_eq!(generations(dir.path())?.len(), 1);
        let expected = assert_current(&core)?;
        drop(handle);
        drop(core);
        let reopened = open(dir.path(), &config)?;
        assert_eq!(assert_current(&reopened)?, expected);
    }
    Ok(())
}

/// Pauses the worker inside a hook until the test releases it.
fn gate() -> (Hook, mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (reached_tx, reached_rx) = mpsc::channel();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let hook: Hook = Box::new(move || {
        reached_tx.send(()).unwrap();
        go_rx.recv().unwrap();
    });
    (hook, reached_rx, go_tx)
}

#[test]
fn cancellation_before_publication_removes_staging_and_keeps_every_write() -> Result<()> {
    for at_publication in [false, true] {
        let dir = tempfile::tempdir()?;
        let core = open(dir.path(), &config())?;
        seed(&core)?;
        let before = generations(dir.path())?.len();
        let (hook, reached, go) = gate();
        let handle = if at_publication {
            core.spawn_compaction(CompactionOptions::default(), none(), hook)?
        } else {
            core.spawn_compaction(CompactionOptions::default(), hook, none())?
        };
        reached.recv()?;
        // The worker is paused holding no lease: foreground writes and queries proceed.
        mutate(&core)?;
        assert_current(&core)?;
        assert_eq!(
            generations(dir.path())?.len(),
            before + 1,
            "staging exists while running"
        );
        handle.cancel();
        handle.cancel();
        assert!(handle.progress().cancel_requested);
        go.send(())?;
        let report = handle.wait();
        assert_eq!(report.status, CompactionStatus::Cancelled, "{report:?}");
        assert!(!report.published);
        assert_eq!(report.reclaimed_bytes, 0);
        assert_eq!(generations(dir.path())?.len(), before);
        let expected = assert_current(&core)?;
        drop(handle);
        drop(core);
        let reopened = open(dir.path(), &config())?;
        assert_eq!(assert_current(&reopened)?, expected);
    }
    Ok(())
}

fn run(dir: &std::path::Path, options: CompactionOptions) -> Result<CompactionReport> {
    let core = open(dir, &config())?;
    let report = core.start_compaction(options)?.wait();
    assert!(generations(dir)?.len() <= 1, "{report:?}");
    assert_current(&core)?;
    Ok(report)
}

fn churned_store() -> Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    let core = open(dir.path(), &config())?;
    seed(&core)?;
    churn(&core)?;
    mutate(&core)?;
    Ok(dir)
}

#[test]
fn memory_budget_boundary_stops_before_staging() -> Result<()> {
    let needed = run(churned_store()?.path(), CompactionOptions::default())?
        .progress
        .snapshot_bytes;
    assert!(needed > 0);
    for (limit, completes) in [(needed - 1, false), (needed, true), (needed + 1, true)] {
        let report = run(
            churned_store()?.path(),
            CompactionOptions {
                max_memory_bytes: Some(limit),
                ..Default::default()
            },
        )?;
        if completes {
            assert_eq!(
                report.status,
                CompactionStatus::Completed,
                "{limit}: {report:?}"
            );
        } else {
            assert_eq!(
                report.status,
                CompactionStatus::BudgetExceeded,
                "{report:?}"
            );
            assert_eq!(report.resource, Some(CompactionResource::Memory));
            assert_eq!(report.progress.frames_written, 0);
            assert!(!report.published);
        }
    }
    Ok(())
}

#[test]
fn temporary_disk_budget_boundary_stops_staging_and_cleans_up() -> Result<()> {
    // The staging generation's first allocation is one growth step, enough for this store.
    let step = crate::core::storage::GROWTH_STEP as u64;
    for (limit, completes) in [
        (1, false),
        (step - 1, false),
        (step, true),
        (step + 1, true),
    ] {
        let report = run(
            churned_store()?.path(),
            CompactionOptions {
                max_temp_disk_bytes: Some(limit),
                ..Default::default()
            },
        )?;
        if completes {
            assert_eq!(
                report.status,
                CompactionStatus::Completed,
                "{limit}: {report:?}"
            );
        } else {
            assert_eq!(
                report.status,
                CompactionStatus::BudgetExceeded,
                "{report:?}"
            );
            assert_eq!(report.resource, Some(CompactionResource::TempDisk));
            assert!(!report.published);
            assert!(report.message.unwrap().contains("temporary-disk"));
        }
    }
    Ok(())
}

#[test]
fn busy_and_no_progress_finish_without_publishing() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let core = open(dir.path(), &config())?;
    seed(&core)?;
    churn(&core)?;
    {
        let _held = core.compaction.lock();
        let report = core.start_compaction(CompactionOptions::default())?.wait();
        assert_eq!(report.status, CompactionStatus::Busy, "{report:?}");
        assert!(!report.published);
    }
    assert_eq!(generations(dir.path())?.len(), 0);
    core.compact()?;
    let live = generations(dir.path())?;
    let report = core.start_compaction(CompactionOptions::default())?.wait();
    assert_eq!(report.status, CompactionStatus::NoProgress, "{report:?}");
    assert!(report.message.unwrap().contains("not smaller"));
    assert_eq!(generations(dir.path())?, live);
    let report = core
        .start_compaction(CompactionOptions {
            require_reclaim: false,
            ..Default::default()
        })?
        .wait();
    assert_eq!(report.status, CompactionStatus::Completed, "{report:?}");
    assert!(core
        .start_compaction(CompactionOptions {
            max_cpu_fraction: 0.0,
            ..Default::default()
        })
        .is_err());
    Ok(())
}

#[test]
fn worker_panic_fails_cleanly_and_dropping_the_handle_releases_the_store() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let core = open(dir.path(), &config())?;
    seed(&core)?;
    let report = core
        .spawn_compaction(
            CompactionOptions::default(),
            Box::new(|| panic!("injected")),
            none(),
        )?
        .wait();
    assert_eq!(report.status, CompactionStatus::Failed);
    assert!(report.message.unwrap().contains("injected"));
    assert_eq!(generations(dir.path())?.len(), 0);

    let (hook, reached, go) = gate();
    let handle = core.spawn_compaction(CompactionOptions::default(), hook, none())?;
    reached.recv()?;
    drop(core);
    // The worker still holds the engine, so the store is locked.
    assert!(WarmCore::new(1024, Some(dir.path().display().to_string()), Some(config())).is_err());
    go.send(())?;
    drop(handle);
    let reopened = open(dir.path(), &config())?;
    assert_eq!(generations(dir.path())?.len(), 0);
    assert_eq!(reopened.vector_count(), 2);
    Ok(())
}

#[test]
fn throttled_compaction_keeps_concurrent_writes_and_queries_usable() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let core = open(dir.path(), &config())?;
    seed(&core)?;
    churn(&core)?;
    let handle = core.start_compaction(CompactionOptions {
        max_cpu_fraction: 0.25,
        ..Default::default()
    })?;
    let mut written = 0;
    while !handle.is_finished() || written < 20 {
        core.memorize(
            format!("during-{written}"),
            crate::EntangledHVec::new_deterministic(1024, 10_000 + written),
        )?;
        assert!(!core.query(&core.encode_text("original"), 1)?.is_empty());
        written += 1;
    }
    let report = handle.wait();
    assert_eq!(report.status, CompactionStatus::Completed, "{report:?}");
    drop(handle);
    drop(core);
    let reopened = open(dir.path(), &config())?;
    for i in 0..written {
        assert!(reopened
            .shards
            .read()
            .get_vector(&format!("during-{i}"))
            .is_some());
    }
    Ok(())
}

#[test]
fn background_crash_child() -> Result<()> {
    let Ok(root) = crate::compatibility::var("WARM_TEST_BACKGROUND_ROOT") else {
        return Ok(());
    };
    let encrypted =
        crate::compatibility::var("WARM_TEST_BACKGROUND_ENCRYPTED").as_deref() == Ok("true");
    let config = configurations()
        .find(|config| config.security.encryption_enabled == encrypted)
        .unwrap();
    let core = open(std::path::Path::new(&root), &config)?;
    seed(&core)?;
    let writer = Arc::clone(&core);
    let report = core
        .spawn_compaction(
            CompactionOptions::default(),
            Box::new(move || mutate(&writer).unwrap()),
            none(),
        )?
        .wait();
    anyhow::bail!("compaction finished without reaching its crash point: {report:?}")
}

/// A background compaction killed at each publish stage reopens with every acknowledged
/// write, and the next compaction leaves only the live generation.
#[test]
fn background_compaction_survives_interruption_at_each_publish_stage() -> Result<()> {
    for config in configurations() {
        for stage in [
            "staging",
            "caught-up",
            "staged",
            "published",
            "mapped",
            "released",
            "finished",
        ] {
            let dir = tempfile::tempdir()?;
            let output = std::process::Command::new(std::env::current_exe()?)
                .args([
                    "--exact",
                    "core::engine::background_compaction::tests::background_crash_child",
                ])
                .env("WARM_TEST_BACKGROUND_ROOT", dir.path())
                .env(
                    "WARM_TEST_BACKGROUND_ENCRYPTED",
                    config.security.encryption_enabled.to_string(),
                )
                .env("WARM_TEST_COMPACTION_PHASE", stage)
                .output()?;
            assert_eq!(output.status.code(), Some(73), "{stage}: {output:?}");
            let core = open(dir.path(), &config)?;
            let expected = assert_current(&core)?;
            core.memorize_document(document("after-recovery", "recovered"))?;
            let report = core
                .start_compaction(CompactionOptions {
                    require_reclaim: false,
                    ..Default::default()
                })?
                .wait();
            assert_eq!(
                report.status,
                CompactionStatus::Completed,
                "{stage}: {report:?}"
            );
            assert_eq!(generations(dir.path())?.len(), 1, "{stage}");
            if matches!(stage, "staging" | "caught-up" | "staged") {
                assert!(report.stale_bytes_removed > 0, "{stage}: {report:?}");
            }
            drop(core);
            let reopened = open(dir.path(), &config)?;
            assert!(reopened.has_document("after-recovery"), "{stage}");
            let _ = expected;
        }
    }
    Ok(())
}

fn percentile(samples: &mut [Duration], p: f64) -> Duration {
    samples.sort_unstable();
    samples[((samples.len() - 1) as f64 * p).round() as usize]
}

/// Foreground write and query latency with and without a background compaction running,
/// paired and alternated over repeats. Run in release:
/// `cargo test --release --lib foreground_latency -- --ignored --nocapture`.
#[test]
#[ignore = "measurement"]
fn foreground_latency_with_and_without_background_compaction() -> Result<()> {
    const REPEATS: usize = 6;
    const OPS: usize = 400;
    let dir = tempfile::tempdir()?;
    let core = open(dir.path(), &config())?;
    for i in 0..20_000u64 {
        core.memorize(
            format!("base-{}", i % 5_000),
            crate::EntangledHVec::new_deterministic(1024, i),
        )?;
    }
    let probe = core.encode_text("probe");
    let mut serial = 0u64;
    for repeat in 0..REPEATS {
        for with_job in [repeat % 2 == 0, repeat % 2 != 0] {
            let start_job = || {
                core.start_compaction(CompactionOptions {
                    require_reclaim: false,
                    ..Default::default()
                })
            };
            let mut job = with_job.then(start_job).transpose()?;
            let (mut writes, mut reads) = (Vec::new(), Vec::new());
            let mut jobs = 0;
            for _ in 0..OPS {
                // Every measured operation overlaps a running compaction in the `job` arm.
                if job.as_ref().is_some_and(CompactionHandle::is_finished) {
                    let report = job.take().unwrap().wait();
                    assert_eq!(report.status, CompactionStatus::Completed, "{report:?}");
                    jobs += 1;
                    job = Some(start_job()?);
                }
                let start = Instant::now();
                core.memorize(
                    format!("fg-{serial}"),
                    crate::EntangledHVec::new_deterministic(1024, 1 << 40 | serial),
                )?;
                writes.push(start.elapsed());
                let start = Instant::now();
                core.query(&probe, 10)?;
                reads.push(start.elapsed());
                serial += 1;
            }
            if let Some(job) = job {
                job.wait();
                jobs += 1;
            }
            println!(
                "repeat={repeat} job={with_job} jobs={jobs} write_p50_us={} write_p99_us={} write_max_us={} query_p50_us={} query_p99_us={}",
                percentile(&mut writes, 0.5).as_micros(),
                percentile(&mut writes, 0.99).as_micros(),
                percentile(&mut writes, 1.0).as_micros(),
                percentile(&mut reads, 0.5).as_micros(),
                percentile(&mut reads, 0.99).as_micros(),
            );
        }
    }
    Ok(())
}
