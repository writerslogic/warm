// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Node surface of background compaction: the same job, options, stages and outcomes as
//! `WarmCore::start_compaction` (`docs/file-format/COMPACTION.md`).

use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::sync::Arc;

use crate::core::engine::{
    CompactionHandle, CompactionOptions, CompactionProgress, CompactionReport,
};

#[napi(object, js_name = "CompactionOptions")]
#[derive(Default)]
pub struct CompactionOptionsJs {
    /// Largest share of one core the worker may use, in (0, 1]. Default 1.
    pub max_cpu_fraction: Option<f64>,
    pub max_memory_bytes: Option<f64>,
    pub max_temp_disk_bytes: Option<f64>,
    /// Publish nothing and finish as `no_progress` when the new generation would not be
    /// smaller. Default true.
    pub require_reclaim: Option<bool>,
}

fn byte_limit(name: &str, value: Option<f64>) -> Result<Option<u64>> {
    match value {
        None => Ok(None),
        Some(v)
            if v.is_finite()
                && v.fract() == 0.0
                && (0.0..=9_007_199_254_740_991.0).contains(&v) =>
        {
            Ok(Some(v as u64))
        }
        Some(_) => Err(Error::from_reason(format!(
            "{name} must be a non-negative safe integer"
        ))),
    }
}

impl CompactionOptionsJs {
    pub(super) fn into_options(self) -> Result<CompactionOptions> {
        let defaults = CompactionOptions::default();
        Ok(CompactionOptions {
            max_cpu_fraction: self.max_cpu_fraction.unwrap_or(defaults.max_cpu_fraction),
            max_memory_bytes: byte_limit("maxMemoryBytes", self.max_memory_bytes)?,
            max_temp_disk_bytes: byte_limit("maxTempDiskBytes", self.max_temp_disk_bytes)?,
            require_reclaim: self.require_reclaim.unwrap_or(defaults.require_reclaim),
        })
    }
}

#[napi(object, js_name = "CompactionProgress")]
pub struct CompactionProgressJs {
    #[napi(
        ts_type = "'queued' | 'snapshot' | 'staging' | 'catching_up' | 'publishing' | 'persisting' | 'finished'"
    )]
    pub stage: String,
    pub frames_total: f64,
    pub frames_written: f64,
    pub tail_frames: f64,
    pub bytes_written: f64,
    pub snapshot_bytes: f64,
    pub throttled_ms: f64,
    pub elapsed_ms: f64,
    pub cancel_requested: bool,
}

#[napi(object, js_name = "CompactionReport")]
pub struct CompactionReportJs {
    #[napi(
        ts_type = "'completed' | 'cancelled' | 'budget_exceeded' | 'busy' | 'no_progress' | 'failed'"
    )]
    pub status: String,
    pub message: Option<String>,
    #[napi(ts_type = "'memory' | 'temp_disk' | null")]
    pub resource: Option<String>,
    pub published: bool,
    pub progress: CompactionProgressJs,
    pub disk_bytes_before: f64,
    pub disk_bytes_after: f64,
    pub reclaimed_bytes: f64,
    pub write_amplification: Option<f64>,
    pub stale_bytes_removed: f64,
}

/// The snake-case name serde gives a unit variant.
fn name<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

impl From<CompactionProgress> for CompactionProgressJs {
    fn from(p: CompactionProgress) -> Self {
        Self {
            stage: name(&p.stage),
            frames_total: p.frames_total as f64,
            frames_written: p.frames_written as f64,
            tail_frames: p.tail_frames as f64,
            bytes_written: p.bytes_written as f64,
            snapshot_bytes: p.snapshot_bytes as f64,
            throttled_ms: p.throttled_ms as f64,
            elapsed_ms: p.elapsed_ms as f64,
            cancel_requested: p.cancel_requested,
        }
    }
}

impl From<CompactionReport> for CompactionReportJs {
    fn from(r: CompactionReport) -> Self {
        Self {
            status: name(&r.status),
            message: r.message,
            resource: r.resource.as_ref().map(name),
            published: r.published,
            progress: r.progress.into(),
            disk_bytes_before: r.disk_bytes_before as f64,
            disk_bytes_after: r.disk_bytes_after as f64,
            reclaimed_bytes: r.reclaimed_bytes as f64,
            write_amplification: r.write_amplification,
            stale_bytes_removed: r.stale_bytes_removed as f64,
        }
    }
}

/// A background compaction started by `WarmMemorySystem.startCompaction`. Collecting it
/// cancels the job; the worker is joined off the JavaScript thread.
#[napi]
pub struct CompactionJob {
    handle: Option<Arc<CompactionHandle>>,
}

impl CompactionJob {
    pub(super) fn new(handle: CompactionHandle) -> Self {
        Self {
            handle: Some(Arc::new(handle)),
        }
    }

    fn handle(&self) -> &Arc<CompactionHandle> {
        self.handle.as_ref().expect("handle present until drop")
    }
}

#[napi]
impl CompactionJob {
    #[napi]
    pub fn progress(&self) -> CompactionProgressJs {
        self.handle().progress().into()
    }

    /// Requests cancellation; idempotent. Ignored once publication has begun.
    #[napi]
    pub fn cancel(&self) {
        self.handle().cancel();
    }

    #[napi(getter)]
    pub fn finished(&self) -> bool {
        self.handle().is_finished()
    }

    /// Resolves with the job's report once the worker has released the store; every call
    /// resolves with the same report.
    #[napi]
    pub async fn wait(&self) -> Result<CompactionReportJs> {
        let handle = Arc::clone(self.handle());
        tokio::task::spawn_blocking(move || handle.wait().into())
            .await
            .map_err(|e| Error::from_reason(e.to_string()))
    }
}

impl Drop for CompactionJob {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.cancel();
            // The last reference joins the worker; keep that off the JavaScript thread. If no
            // thread can be spawned, the closure and the handle drop here and join inline.
            if Arc::strong_count(&handle) == 1 && !handle.is_finished() {
                let _ = std::thread::Builder::new()
                    .name("warm-compaction-release".into())
                    .spawn(move || drop(handle));
            }
        }
    }
}
