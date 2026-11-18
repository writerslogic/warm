// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// On unless `WARM_QGRAPH_HUGEPAGES` is set to 0: the large arrays of an index built in this
/// process or opened onto the heap (code rows, neighbour lists, scales, norm terms, residual
/// rows, neighbour blocks) are advised into transparent huge pages before they are first
/// touched, so the random row fetches of a search share one TLB entry per 2 MiB instead of
/// needing one per 4 KiB (on x86 a page walk per fetch once the index outgrows the TLB's
/// reach). Linux only; a no-op elsewhere, and where transparent huge pages are disabled.
/// Read once.
pub(super) fn huge_pages() -> bool {
    static HUGE: OnceLock<bool> = OnceLock::new();
    *HUGE.get_or_init(|| env_knob::<u8>("WARM_QGRAPH_HUGEPAGES").unwrap_or(1) != 0)
}

/// Huge-page advice given by this process (see [`huge_page_status`]).
pub(super) static HUGE_ADVISED_BYTES: AtomicU64 = AtomicU64::new(0);
pub(super) static HUGE_ADVICE_FAILED: AtomicU64 = AtomicU64::new(0);
/// Test hook: treat every `madvise` as failed, the result under THP `never`, a seccomp filter
/// or a sandbox that rejects the call.
#[cfg(test)]
pub(super) static FAIL_HUGE_ADVICE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// What the huge-page advice of this process achieved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct HugePageStatus {
    /// The advice is on (`WARM_QGRAPH_HUGEPAGES` unset or non-zero).
    pub enabled: bool,
    /// Bytes for which `madvise(MADV_HUGEPAGE)` succeeded.
    pub advised_bytes: u64,
    /// `madvise` calls that failed (each falls back to base pages; nothing else changes).
    pub failed_calls: u64,
    /// `AnonHugePages` of the whole process from `/proc/self/smaps_rollup`, whoever asked for
    /// them (the allocator may hold huge pages of its own). `None` where the file is
    /// unreadable (outside Linux, or a sandbox without it). [`VGraph::code_rows_huge_bytes`]
    /// gives the figure for one index's code rows.
    pub anon_huge_bytes: Option<u64>,
}

/// The huge-page advice given so far and the huge pages the process holds.
pub fn huge_page_status() -> HugePageStatus {
    let anon_huge_bytes = std::fs::read_to_string("/proc/self/smaps_rollup")
        .ok()
        .and_then(|text| {
            let line = text.lines().find(|l| l.starts_with("AnonHugePages:"))?;
            let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
            Some(kib * 1024)
        });
    HugePageStatus {
        enabled: huge_pages(),
        advised_bytes: HUGE_ADVISED_BYTES.load(Ordering::Relaxed),
        failed_calls: HUGE_ADVICE_FAILED.load(Ordering::Relaxed),
        anon_huge_bytes,
    }
}

/// `AnonHugePages` of the mappings of `/proc/self/smaps` that overlap `[ptr, ptr + len)`;
/// `None` where the file is unreadable. A mapping may hold other allocations too, so this
/// bounds the huge pages behind the range from above.
pub(super) fn huge_bytes_behind(ptr: *const u8, len: usize) -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/smaps").ok()?;
    let (lo, hi) = (ptr as u64, ptr as u64 + len as u64);
    let mut total = 0u64;
    let mut inside = false;
    for line in text.lines() {
        let head = line.split_whitespace().next().unwrap_or("");
        if let Some((a, b)) = head.split_once('-') {
            if let (Ok(a), Ok(b)) = (u64::from_str_radix(a, 16), u64::from_str_radix(b, 16)) {
                inside = a < hi && lo < b;
                continue;
            }
        }
        if inside {
            if let Some(rest) = line.strip_prefix("AnonHugePages:") {
                let kib: u64 = rest.split_whitespace().next()?.parse().ok()?;
                total += kib * 1024;
            }
        }
    }
    Some(total)
}

/// Asks the kernel to back the 2 MiB-aligned interior of `[ptr, ptr + len)` with transparent
/// huge pages (Linux `MADV_HUGEPAGE`). Must run before the pages are first touched, so the
/// page-fault path allocates them huge where THP is `always` or `madvise`; afterwards only
/// `khugepaged` could collapse them, slowly. A no-op outside Linux. A failure (THP `never`,
/// a kernel without THP, a filtered syscall) is counted and otherwise ignored: the advice
/// changes paging policy only, never the mapping or its contents.
pub(super) fn advise_huge_pages(ptr: *const u8, len: usize) {
    #[cfg(target_os = "linux")]
    {
        const HUGE: usize = 2 << 20;
        const MADV_HUGEPAGE: i32 = 14;
        extern "C" {
            fn madvise(addr: *mut std::ffi::c_void, len: usize, advice: i32) -> i32;
        }
        let start = (ptr as usize).next_multiple_of(HUGE);
        let end = (ptr as usize + len) & !(HUGE - 1);
        if start < end {
            #[cfg(test)]
            if FAIL_HUGE_ADVICE.load(Ordering::Relaxed) {
                HUGE_ADVICE_FAILED.fetch_add(1, Ordering::Relaxed);
                return;
            }
            // SAFETY: `[start, end)` lies inside the live allocation `[ptr, ptr + len)` and
            // `madvise` with `MADV_HUGEPAGE` reads or writes nothing in it.
            let rc = unsafe { madvise(start as *mut std::ffi::c_void, end - start, MADV_HUGEPAGE) };
            if rc == 0 {
                HUGE_ADVISED_BYTES.fetch_add((end - start) as u64, Ordering::Relaxed);
            } else {
                HUGE_ADVICE_FAILED.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (ptr, len);
}

/// [`advise_huge_pages`] over the capacity of a freshly allocated, still untouched vector when
/// [`huge_pages`] is on.
pub(super) fn huge_capacity<T>(ptr: *const T, capacity: usize) {
    if huge_pages() {
        advise_huge_pages(ptr.cast::<u8>(), capacity * std::mem::size_of::<T>());
    }
}
