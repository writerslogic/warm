//! Global allocator of the Python extension. It forwards to the system allocator (or
//! mimalloc under the `mimalloc` feature) and, on Linux when `WARM_THP=1`, asks the kernel to
//! back every allocation of at least 2 MiB with transparent huge pages (`madvise(MADV_HUGEPAGE)`).
//! The index keeps multi-GB row and code arrays that the scan and reorder phases touch at random,
//! where 4 KiB pages cost a TLB miss per access.
//!
//! The mode is read once from the environment with `getenv` (no allocation) on first use.
use std::alloc::{GlobalAlloc, Layout, System};
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU8, Ordering};

#[cfg(feature = "mimalloc")]
type Inner = mimalloc::MiMalloc;
#[cfg(not(feature = "mimalloc"))]
type Inner = System;

pub struct WarmAlloc;

#[cfg(feature = "mimalloc")]
static INNER: Inner = mimalloc::MiMalloc;
#[cfg(not(feature = "mimalloc"))]
static INNER: Inner = System;

#[cfg(target_os = "linux")]
const UNSET: u8 = 2;
#[cfg(target_os = "linux")]
static THP: AtomicU8 = AtomicU8::new(UNSET);
/// THP mode when `WARM_THP` is unset.
#[cfg(target_os = "linux")]
const THP_DEFAULT: u8 = 0;
#[cfg(target_os = "linux")]
const HUGE: usize = 2 << 20;

#[cfg(target_os = "linux")]
fn thp_enabled() -> bool {
    let mut v = THP.load(Ordering::Relaxed);
    if v == UNSET {
        v = THP_DEFAULT;
        {
            // SAFETY: getenv is given a NUL-terminated name and the result is read at once.
            let p = unsafe { libc::getenv(c"WARM_THP".as_ptr()) };
            if !p.is_null() {
                // SAFETY: p points at a NUL-terminated string owned by the environment.
                v = u8::from(unsafe { *p } as u8 == b'1');
            }
        }
        THP.store(v, Ordering::Relaxed);
    }
    v == 1
}

#[cfg(target_os = "linux")]
fn advise(p: *mut u8, size: usize) {
    if p.is_null() || size < HUGE || !thp_enabled() {
        return;
    }
    let start = (p as usize + HUGE - 1) & !(HUGE - 1);
    let end = (p as usize + size) & !(HUGE - 1);
    if end > start {
        // SAFETY: [start, end) lies inside the allocation just returned; the advice is a hint
        // that never changes contents and whose failure is ignored.
        unsafe { libc::madvise(start as *mut libc::c_void, end - start, libc::MADV_HUGEPAGE) };
    }
}

#[cfg(not(target_os = "linux"))]
fn advise(_: *mut u8, _: usize) {}

// SAFETY: every method forwards to the inner allocator with the caller's layout and only adds a
// madvise hint on the returned range.
unsafe impl GlobalAlloc for WarmAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = INNER.alloc(l);
        advise(p, l.size());
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = INNER.alloc_zeroed(l);
        advise(p, l.size());
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        INNER.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let q = INNER.realloc(p, l, new);
        advise(q, new);
        q
    }
}
