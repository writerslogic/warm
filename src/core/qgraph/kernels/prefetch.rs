// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

/// Hint the cache to load every line of `bytes` (128-byte lines on Apple cores).
#[inline]
pub(crate) fn prefetch_bytes(bytes: &[i8]) {
    #[cfg(target_arch = "aarch64")]
    if let Some(last) = bytes.len().checked_sub(1) {
        let base = bytes.as_ptr();
        let mut off = 0;
        loop {
            let at = off.min(last);
            // SAFETY: PRFM is a hint that never faults or writes; `at < bytes.len()`, so the
            // address is inside the live slice.
            unsafe {
                std::arch::asm!(
                    "prfm pldl1keep, [{0}]",
                    in(reg) base.add(at),
                    options(nostack, preserves_flags, readonly)
                );
            }
            if at == last {
                break;
            }
            off += 128;
        }
    }
    #[cfg(target_arch = "x86_64")]
    for line in bytes.chunks(64) {
        // SAFETY: PREFETCHT0 is a hint that never faults or writes; the address is inside
        // the live slice.
        unsafe {
            std::arch::x86_64::_mm_prefetch::<{ std::arch::x86_64::_MM_HINT_T0 }>(
                line.as_ptr().cast::<i8>(),
            );
        }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    let _ = bytes;
}

/// Hint the cache to load `words`. A no-op where no prefetch instruction is wired up.
#[inline]
pub(crate) fn prefetch(words: &[u64]) {
    #[cfg(target_arch = "aarch64")]
    for line in words.chunks(16) {
        // SAFETY: PRFM is a hint: it never faults, writes no register or memory, and the
        // address points into a live slice.
        unsafe {
            std::arch::asm!(
                "prfm pldl1keep, [{0}]",
                in(reg) line.as_ptr(),
                options(nostack, preserves_flags, readonly)
            );
        }
    }
    #[cfg(target_arch = "x86_64")]
    for line in words.chunks(8) {
        // SAFETY: as in `prefetch_bytes`.
        unsafe {
            std::arch::x86_64::_mm_prefetch::<{ std::arch::x86_64::_MM_HINT_T0 }>(
                line.as_ptr().cast::<i8>(),
            );
        }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    let _ = words;
}
