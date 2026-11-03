// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Compaction on a worker thread with progress, cancellation and budgets
//! (`docs/file-format/COMPACTION.md`). The work is `compact_impl`, shared with `compact()`.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{ensure, Result};
use serde::Serialize;

use super::WarmCore;

/// Limits for one background compaction. `None` means unbounded.
#[derive(Clone, Debug, PartialEq)]
pub struct CompactionOptions {
    /// Largest share of one core the worker may use, in `(0, 1]`.
    pub max_cpu_fraction: f64,
    /// Bound on the estimated bytes of the cloned snapshot.
    pub max_memory_bytes: Option<u64>,
    /// Bound on the allocated file length of the staging generation.
    pub max_temp_disk_bytes: Option<u64>,
    /// Finish as `no_progress`, publishing nothing, when the new generation would not be
    /// smaller than the live one.
    pub require_reclaim: bool,
}

impl Default for CompactionOptions {
    fn default() -> Self {
        Self {
            max_cpu_fraction: 1.0,
            max_memory_bytes: None,
            max_temp_disk_bytes: None,
            require_reclaim: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionStage {
    Queued,
    Snapshot,
    Staging,
    CatchingUp,
    Publishing,
    Persisting,
    Finished,
}

const STAGES: [CompactionStage; 7] = [
    CompactionStage::Queued,
    CompactionStage::Snapshot,
    CompactionStage::Staging,
    CompactionStage::CatchingUp,
    CompactionStage::Publishing,
    CompactionStage::Persisting,
    CompactionStage::Finished,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionStatus {
    Completed,
    Cancelled,
    BudgetExceeded,
    Busy,
    NoProgress,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionResource {
    Memory,
    TempDisk,
}

impl std::fmt::Display for CompactionResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Memory => "memory",
            Self::TempDisk => "temporary-disk",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionProgress {
    pub stage: CompactionStage,
    pub frames_total: u64,
    pub frames_written: u64,
    pub tail_frames: u64,
    pub bytes_written: u64,
    pub snapshot_bytes: u64,
    pub throttled_ms: u64,
    pub elapsed_ms: u64,
    pub cancel_requested: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionReport {
    pub status: CompactionStatus,
    /// The error or the reason for a status other than `completed`.
    pub message: Option<String>,
    /// The budget that stopped a `budget_exceeded` job.
    pub resource: Option<CompactionResource>,
    /// Whether the new generation replaced the live one.
    pub published: bool,
    pub progress: CompactionProgress,
    pub disk_bytes_before: u64,
    pub disk_bytes_after: u64,
    pub reclaimed_bytes: u64,
    /// `bytes_written / reclaimed_bytes`; `None` when nothing was reclaimed.
    pub write_amplification: Option<f64>,
    pub stale_bytes_removed: u64,
}

/// Why a compaction stopped without publishing, as an error `compact_impl` returns.
#[derive(Debug, thiserror::Error)]
pub(super) enum Stop {
    #[error("compaction cancelled")]
    Cancelled,
    #[error("compaction {resource} budget exceeded: {required} bytes needed, limit {limit}")]
    Budget {
        resource: CompactionResource,
        limit: u64,
        required: u64,
    },
    #[error("another compaction is running")]
    Busy,
    #[error("compaction cannot make progress: {0}")]
    NoProgress(String),
}

/// The shared state of one compaction: options, cancellation, progress counters and, for a
/// background job, its result.
pub(super) struct JobState {
    options: CompactionOptions,
    background: bool,
    cancel: AtomicBool,
    stage: AtomicU8,
    pub(super) frames_total: AtomicU64,
    pub(super) frames_written: AtomicU64,
    pub(super) tail_frames: AtomicU64,
    pub(super) bytes_written: AtomicU64,
    pub(super) snapshot_bytes: AtomicU64,
    throttled_ns: AtomicU64,
    pub(super) stale_bytes: AtomicU64,
    pub(super) disk_before: AtomicU64,
    pub(super) disk_after: AtomicU64,
    pub(super) published: AtomicBool,
    started: Instant,
    slice: Mutex<Instant>,
    result: Mutex<Option<CompactionReport>>,
    done: Condvar,
}

impl JobState {
    fn new(options: CompactionOptions, background: bool) -> Self {
        let now = Instant::now();
        Self {
            options,
            background,
            cancel: AtomicBool::new(false),
            stage: AtomicU8::new(0),
            frames_total: AtomicU64::new(0),
            frames_written: AtomicU64::new(0),
            tail_frames: AtomicU64::new(0),
            bytes_written: AtomicU64::new(0),
            snapshot_bytes: AtomicU64::new(0),
            throttled_ns: AtomicU64::new(0),
            stale_bytes: AtomicU64::new(0),
            disk_before: AtomicU64::new(0),
            disk_after: AtomicU64::new(0),
            published: AtomicBool::new(false),
            started: now,
            slice: Mutex::new(now),
            result: Mutex::new(None),
            done: Condvar::new(),
        }
    }

    /// The unbounded, uncancellable state `compact()` runs with.
    pub(super) fn foreground() -> Self {
        Self::new(
            CompactionOptions {
                require_reclaim: false,
                ..CompactionOptions::default()
            },
            false,
        )
    }

    pub(super) fn background(&self) -> bool {
        self.background
    }

    pub(super) fn require_reclaim(&self) -> bool {
        self.options.require_reclaim
    }

    pub(super) fn set_stage(&self, stage: CompactionStage) {
        let index = STAGES.iter().position(|s| *s == stage).unwrap_or(0);
        self.stage.store(index as u8, Ordering::Release);
        self.begin_slice();
    }

    pub(super) fn check_cancel(&self) -> Result<()> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(Stop::Cancelled.into());
        }
        Ok(())
    }

    /// Fails when the staging generation would grow past the temporary-disk budget.
    pub(super) fn check_disk(&self, predicted: usize, extra: usize) -> Result<()> {
        if let Some(limit) = self.options.max_temp_disk_bytes {
            let required = (predicted as u64).saturating_add(extra as u64);
            if required > limit {
                return Err(Stop::Budget {
                    resource: CompactionResource::TempDisk,
                    limit,
                    required,
                }
                .into());
            }
        }
        Ok(())
    }

    /// Adds `bytes` to the snapshot estimate and fails once it passes the memory budget.
    pub(super) fn account_memory(&self, total: &mut u64, bytes: usize) -> Result<()> {
        *total = total.saturating_add(bytes as u64);
        self.snapshot_bytes.store(*total, Ordering::Relaxed);
        if let Some(limit) = self.options.max_memory_bytes {
            if *total > limit {
                return Err(Stop::Budget {
                    resource: CompactionResource::Memory,
                    limit,
                    required: *total,
                }
                .into());
            }
        }
        Ok(())
    }

    /// Starts a new throttled work slice; call it after any wait the throttle must not count.
    pub(super) fn begin_slice(&self) {
        *self.slice.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }

    /// Sleeps in proportion to the work slice just done to hold the CPU share at the budget.
    /// The caller holds no lock.
    pub(super) fn throttle(&self) {
        const SLICE: Duration = Duration::from_millis(5);
        let fraction = self.options.max_cpu_fraction;
        if fraction >= 1.0 {
            return;
        }
        let elapsed = self
            .slice
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .elapsed();
        if elapsed < SLICE {
            return;
        }
        let pause = elapsed.mul_f64(1.0 / fraction - 1.0);
        std::thread::sleep(pause);
        self.throttled_ns.fetch_add(
            u64::try_from(pause.as_nanos()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        self.begin_slice();
    }

    pub(super) fn progress(&self) -> CompactionProgress {
        CompactionProgress {
            stage: STAGES[usize::from(self.stage.load(Ordering::Acquire)).min(STAGES.len() - 1)],
            frames_total: self.frames_total.load(Ordering::Relaxed),
            frames_written: self.frames_written.load(Ordering::Relaxed),
            tail_frames: self.tail_frames.load(Ordering::Relaxed),
            bytes_written: self.bytes_written.load(Ordering::Relaxed),
            snapshot_bytes: self.snapshot_bytes.load(Ordering::Relaxed),
            throttled_ms: self.throttled_ns.load(Ordering::Relaxed) / 1_000_000,
            elapsed_ms: u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            cancel_requested: self.cancel.load(Ordering::Acquire),
        }
    }

    pub(super) fn report(&self, outcome: &Result<()>) -> CompactionReport {
        let published = self.published.load(Ordering::Acquire);
        let (status, resource) = match outcome {
            Ok(()) => (CompactionStatus::Completed, None),
            Err(error) => match error.downcast_ref::<Stop>() {
                _ if published => (CompactionStatus::Failed, None),
                Some(Stop::Cancelled) => (CompactionStatus::Cancelled, None),
                Some(Stop::Budget { resource, .. }) => {
                    (CompactionStatus::BudgetExceeded, Some(*resource))
                }
                Some(Stop::Busy) => (CompactionStatus::Busy, None),
                Some(Stop::NoProgress(_)) => (CompactionStatus::NoProgress, None),
                None => (CompactionStatus::Failed, None),
            },
        };
        let progress = self.progress();
        let (before, after) = if published {
            (
                self.disk_before.load(Ordering::Acquire),
                self.disk_after.load(Ordering::Acquire),
            )
        } else {
            (0, 0)
        };
        let reclaimed = before.saturating_sub(after);
        CompactionReport {
            status,
            message: outcome.as_ref().err().map(|error| format!("{error:#}")),
            resource,
            published,
            write_amplification: (reclaimed > 0)
                .then(|| progress.bytes_written as f64 / reclaimed as f64),
            progress,
            disk_bytes_before: before,
            disk_bytes_after: after,
            reclaimed_bytes: reclaimed,
            stale_bytes_removed: self.stale_bytes.load(Ordering::Acquire),
        }
    }

    fn finish(&self, report: CompactionReport) {
        *self.result.lock().unwrap_or_else(PoisonError::into_inner) = Some(report);
        self.done.notify_all();
    }

    fn wait(&self) -> CompactionReport {
        let mut result = self.result.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(report) = result.as_ref() {
                return report.clone();
            }
            result = self
                .done
                .wait(result)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// A running or finished background compaction. Dropping it cancels the job and waits for the
/// worker, so the engine (and its store lock) is not held past the handle.
pub struct CompactionHandle {
    job: Arc<JobState>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl CompactionHandle {
    pub fn progress(&self) -> CompactionProgress {
        self.job.progress()
    }

    /// Requests cancellation; idempotent. Honoured until the final publication step begins,
    /// after which the job completes and reports `cancel_requested`.
    pub fn cancel(&self) {
        self.job.cancel.store(true, Ordering::Release);
    }

    pub fn is_finished(&self) -> bool {
        self.job
            .result
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    /// Blocks until the job finishes and returns its report; any number of calls return the
    /// same report. When it returns, the worker no longer holds the engine.
    pub fn wait(&self) -> CompactionReport {
        self.job.wait()
    }
}

impl Drop for CompactionHandle {
    fn drop(&mut self) {
        self.cancel();
        if let Some(worker) = self.worker.take() {
            // The worker body catches panics, so a join error cannot carry one.
            let _ = worker.join();
        }
    }
}

type Hook = Box<dyn FnOnce() + Send>;

impl WarmCore {
    /// Starts a compaction on a worker thread (`docs/file-format/COMPACTION.md`). Foreground
    /// reads and writes continue while it runs; writes wait only while it snapshots and
    /// publishes.
    pub fn start_compaction(
        self: &Arc<Self>,
        options: CompactionOptions,
    ) -> Result<CompactionHandle> {
        self.spawn_compaction(options, Box::new(|| {}), Box::new(|| {}))
    }

    pub(super) fn spawn_compaction(
        self: &Arc<Self>,
        options: CompactionOptions,
        after_snapshot: Hook,
        before_publication: Hook,
    ) -> Result<CompactionHandle> {
        ensure!(
            options.max_cpu_fraction.is_finite()
                && options.max_cpu_fraction > 0.0
                && options.max_cpu_fraction <= 1.0,
            "maxCpuFraction must be in (0, 1]"
        );
        let job = Arc::new(JobState::new(options, true));
        let core = Arc::clone(self);
        let state = Arc::clone(&job);
        let worker = std::thread::Builder::new()
            .name("warm-compaction".into())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    core.compact_impl(after_snapshot, before_publication, false, &state)
                }))
                .unwrap_or_else(|panic| {
                    let reason = panic
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_owned())
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown panic".into());
                    Err(anyhow::anyhow!("compaction worker panicked: {reason}"))
                });
                state.set_stage(CompactionStage::Finished);
                let report = state.report(&outcome);
                // Released before the report is visible, so `wait` returning means the store
                // lock is no longer held on this thread's behalf.
                drop(core);
                state.finish(report);
            })?;
        Ok(CompactionHandle {
            job,
            worker: Some(worker),
        })
    }
}

#[cfg(test)]
mod tests;
