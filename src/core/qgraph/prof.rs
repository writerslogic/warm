// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Opt-in construction profile (`WARM_QGRAPH_BUILD_PROFILE` set): per-phase wall and CPU
//! seconds, construction-search filter counts and a hash of the built graph, on stderr.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::OnceLock;
use std::time::Instant;

pub(super) static GREEDY: AtomicU64 = AtomicU64::new(0);
pub(super) static PRUNE: AtomicU64 = AtomicU64::new(0);
pub(super) static REV: AtomicU64 = AtomicU64::new(0);
pub(super) static FRESH: AtomicU64 = AtomicU64::new(0);
pub(super) static EXACT: AtomicU64 = AtomicU64::new(0);
/// Called with each phase's name as it ends (see [`super::set_build_phase_hook`]).
pub(super) static HOOK: OnceLock<fn(&str)> = OnceLock::new();

pub(super) fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        HOOK.get().is_some() || crate::compatibility::var_os("WARM_QGRAPH_BUILD_PROFILE").is_some()
    })
}

pub(super) fn start() -> Option<Instant> {
    on().then(Instant::now)
}

pub(super) fn add(c: &AtomicU64, t: Option<Instant>) {
    if let Some(t) = t {
        c.fetch_add(t.elapsed().as_nanos() as u64, Relaxed);
    }
}

pub(super) fn count(c: &AtomicU64, n: u64) {
    if on() {
        c.fetch_add(n, Relaxed);
    }
}

pub(super) fn report(phase: &str, t: Option<Instant>) {
    let Some(t) = t else { return };
    if let Some(hook) = HOOK.get() {
        hook(phase);
    }
    let secs = |c: &AtomicU64| c.swap(0, Relaxed) as f64 * 1e-9;
    eprintln!(
        "PHASE {phase} wall={:.2}s greedy_cpu={:.1}s prune_cpu={:.1}s rev_wall={:.2}s fresh={} exact={}",
        t.elapsed().as_secs_f64(),
        secs(&GREEDY),
        secs(&PRUNE),
        secs(&REV),
        FRESH.swap(0, Relaxed),
        EXACT.swap(0, Relaxed),
    );
}

pub(super) fn graph_hash(h: impl std::hash::Hash) {
    if on() {
        use std::hash::Hasher;
        let mut s = std::collections::hash_map::DefaultHasher::new();
        h.hash(&mut s);
        eprintln!("GRAPH_HASH {:016x}", s.finish());
    }
}
