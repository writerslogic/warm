// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! The x86 kernel tier, detected once. `WARM_QGRAPH_KERNEL=scalar|avx2|avxvnni|vnni` (read
//! once, at the first distance call) pins a lower tier than the CPU supports, so every tier can
//! be run and compared on one machine; `auto` or unset takes the best detected tier, and a tier
//! the CPU does not support falls back to the best detected one below it. `avxvnni` is the
//! VEX-encoded 256-bit `vpdpbusd` (client cores without AVX-512), `vnni` the AVX512-VNNI one;
//! the per-query kernels ([`dot_i8_query`] and the 4-bit ones) use [`super::vnni`] on both.
//! The 1-bit screen follows the pin: `scalar` takes `count_ones`, any AVX2 tier the AVX2
//! popcount from [`AVX2_SCREEN_MIN_WORDS`] words and POPCNT below.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

pub(super) const SCALAR: u8 = 1;
pub(super) const AVX2: u8 = 2;
pub(super) const AVXVNNI: u8 = 3;
pub(super) const VNNI: u8 = 4;
static TIER: AtomicU8 = AtomicU8::new(0);
/// `popcnt` detected and the tier not pinned to `scalar`. Stored before [`TIER`]; a reader
/// that sees the tier but not yet this flag takes the scalar popcount, which is exact too.
static POPCNT: AtomicBool = AtomicBool::new(false);
/// `avx2` and `popcnt` detected and the resolved tier at least [`AVX2`]: the 1-bit screen
/// may take the AVX2 kernel. Stored before [`TIER`] like [`POPCNT`]; an early reader takes
/// POPCNT or scalar, both exact.
static SCREEN_AVX2: AtomicBool = AtomicBool::new(false);

#[inline(always)]
pub(super) fn get() -> u8 {
    match TIER.load(Ordering::Relaxed) {
        0 => detect(),
        t => t,
    }
}

/// True if the popcount kernels may use the POPCNT instruction.
#[inline(always)]
pub(super) fn popcnt() -> bool {
    get();
    POPCNT.load(Ordering::Relaxed)
}

/// True if the 1-bit screen may use the AVX2 popcount kernel.
#[inline(always)]
pub(super) fn screen_avx2() -> bool {
    get();
    SCREEN_AVX2.load(Ordering::Relaxed)
}

#[cold]
fn detect() -> u8 {
    let avx2 = is_x86_feature_detected!("avx2");
    let vnni = avx2
        && is_x86_feature_detected!("avx512f")
        && is_x86_feature_detected!("avx512vnni")
        && is_x86_feature_detected!("avx512vl");
    let avxvnni = avx2 && is_x86_feature_detected!("avxvnni");
    let supported = |t: u8| match t {
        VNNI => vnni,
        AVXVNNI => avxvnni,
        AVX2 => avx2,
        _ => true,
    };
    let best = [VNNI, AVXVNNI, AVX2, SCALAR]
        .into_iter()
        .find(|&t| supported(t))
        .unwrap_or(SCALAR);
    let want = match crate::compatibility::var("WARM_QGRAPH_KERNEL").as_deref() {
        Ok("scalar") => SCALAR,
        Ok("avx2") => AVX2,
        Ok("avxvnni") => AVXVNNI,
        Ok("vnni") => VNNI,
        Ok("auto") | Err(_) => best,
        Ok(other) => {
            eprintln!(
                "WARM_QGRAPH_KERNEL={other:?} is not scalar|avx2|avxvnni|vnni|auto; using auto"
            );
            best
        }
    };
    // The highest supported tier at or below the one asked for.
    let tier = (SCALAR..=want.min(VNNI))
        .rev()
        .find(|&t| supported(t))
        .unwrap_or(SCALAR);
    let popcnt = want != SCALAR && is_x86_feature_detected!("popcnt");
    POPCNT.store(popcnt, Ordering::Relaxed);
    SCREEN_AVX2.store(popcnt && tier >= AVX2, Ordering::Relaxed);
    TIER.store(tier, Ordering::Relaxed);
    tier
}
