// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! The tuned mimalloc the shipped artifacts (`warm-server`, `warm-mcp`, `warm-admin` and the Python
//! wheel) install as their global allocator. The library never installs one: a binary declares
//!
//! ```ignore
//! #[global_allocator]
//! static GLOBAL: writerslogic_warm::allocator::Tuned = writerslogic_warm::allocator::Tuned;
//! ```
//!
//! behind the `mimalloc` feature.
//!
//! Ingest allocates a term table per chunk, and mimalloc serves that faster than the system
//! allocator. Its defaults commit whole arenas and purge freed pages after one second, which holds a
//! build's peak resident size at about twice the system allocator's. [`Tuned`] commits arenas on demand
//! and purges freed pages at once, which keeps the speed and the footprint (`MIMALLOC_ARENA_EAGER_COMMIT=0
//! MIMALLOC_PURGE_DELAY=0` does the same from the environment).

use std::alloc::{GlobalAlloc, Layout};
use std::os::raw::{c_int, c_long};
use std::sync::atomic::{AtomicBool, Ordering};

// The option numbers are those of the mimalloc 2 header that `libmimalloc-sys` 0.1 vendors on every
// platform; the defaults are checked first, so a different numbering leaves the allocator alone.
const ARENA_EAGER_COMMIT: c_int = 4;
const PURGE_DELAY: c_int = 15;
const PURGE_AT_ONCE: c_long = 0;

extern "C" {
    fn mi_option_get(option: c_int) -> c_long;
    fn mi_option_set(option: c_int, value: c_long);
}

/// [`mimalloc::MiMalloc`] with the arena and purge options set before its first allocation.
pub struct Tuned;

static TUNED: AtomicBool = AtomicBool::new(false);

#[cold]
fn tune() {
    if !TUNED.swap(true, Ordering::Relaxed) {
        // SAFETY: both functions are mimalloc's option accessors. The arena option is read when the
        // first arena is reserved, which has not happened while this runs ahead of the first
        // allocation; a later call only changes options that are read on use.
        unsafe {
            if mi_option_get(ARENA_EAGER_COMMIT) == 2 && mi_option_get(PURGE_DELAY) == 1000 {
                mi_option_set(ARENA_EAGER_COMMIT, 0);
                mi_option_set(PURGE_DELAY, PURGE_AT_ONCE);
            }
        }
    }
}

/// The options in effect: `(arena_eager_commit, purge_delay_ms)`.
pub fn options() -> (i64, i64) {
    // SAFETY: read-only option accessor.
    unsafe {
        (
            mi_option_get(ARENA_EAGER_COMMIT) as i64,
            mi_option_get(PURGE_DELAY) as i64,
        )
    }
}

// SAFETY: every method forwards to `mimalloc::MiMalloc`, which implements `GlobalAlloc`; `tune` only
// sets allocator options and does not allocate.
unsafe impl GlobalAlloc for Tuned {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !TUNED.load(Ordering::Relaxed) {
            tune();
        }
        mimalloc::MiMalloc.alloc(layout)
    }
    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        mimalloc::MiMalloc.dealloc(ptr, layout)
    }
    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if !TUNED.load(Ordering::Relaxed) {
            tune();
        }
        mimalloc::MiMalloc.alloc_zeroed(layout)
    }
    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        mimalloc::MiMalloc.realloc(ptr, layout, new_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test binary keeps the system allocator, so the first allocation through `Tuned` is the
    /// first one mimalloc sees only if nothing else touched the options: the check is on the options
    /// the wrapper leaves behind, which is what the shipped binaries rely on.
    #[test]
    fn first_allocation_sets_the_footprint_options() {
        let layout = Layout::from_size_align(64, 8).unwrap();
        // SAFETY: allocated and freed once with the same layout.
        unsafe {
            let ptr = Tuned.alloc(layout);
            assert!(!ptr.is_null());
            Tuned.dealloc(ptr, layout);
        }
        assert_eq!(options().1, 0);
    }
}
