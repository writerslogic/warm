//! The count-only filter of a conjunction's candidates through a decoded list.

use super::CHECKPOINT_BLOCK;
use crate::core::documents::lists::Cursor;

/// [`super::keep_common`] for candidates that carry no rows and record nothing: the candidates among
/// `slots[..n]` that `other` holds, kept in place. A block with many candidates is matched
/// with [`merge_hits`], which compares eight slots against eight at a time where it can.
#[inline(never)]
pub(super) fn keep_probed(
    other: &mut Cursor<'_>,
    slots: &mut [u32; CHECKPOINT_BLOCK],
    n: usize,
) -> (usize, bool) {
    let (mut i, mut k) = (0usize, 0usize);
    while i < n {
        let wanted = slots[i];
        if other.block_last() < wanted && !other.advance_block(wanted) {
            return (k, false);
        }
        let first = other.block_first();
        if first > wanted {
            while i < n && slots[i] < first {
                i += 1;
            }
            continue;
        }
        let last = other.block_last();
        let block = other.block_slots();
        let mut end = i + 1;
        while end < n && slots[end] <= last {
            end += 1;
        }
        if (end - i) * 16 < block.len() {
            let mut j = 0usize;
            for at in i..end {
                let value = slots[at];
                let mut size = block.len() - j;
                while size > 1 {
                    let half = size / 2;
                    j += half * usize::from(block[j + half] <= value);
                    size -= half;
                }
                slots[k] = value;
                k += usize::from(block[j] == value);
            }
        } else {
            let hits = merge_hits(&slots[i..end], block);
            for (at, bit) in (i..end).zip(0u32..) {
                slots[k] = slots[at];
                k += (hits >> bit) as usize & 1;
            }
        }
        i = end;
    }
    (k, true)
}

/// Bit `t` of the result is set when `block` holds `candidates[t]`. Both ascend; there are at
/// most [`CHECKPOINT_BLOCK`] of each.
#[inline(always)]
pub(super) fn merge_hits(candidates: &[u32], block: &[u32]) -> u128 {
    debug_assert!(candidates.len() <= 128 && block.len() <= 128);
    #[cfg(target_arch = "x86_64")]
    if candidates.len() >= 8 && block.len() >= 8 && is_x86_feature_detected!("avx2") {
        // SAFETY: AVX2 was detected; every load reads eight elements inside the slices.
        return unsafe { merge_hits_avx2(candidates, block) };
    }
    merge_hits_scalar(candidates, block, 0, 0, 0)
}

/// [`merge_hits`] from `candidates[at..]` and `block[from..]` on, adding to `hits`.
#[inline(never)]
pub(super) fn merge_hits_scalar(
    candidates: &[u32],
    block: &[u32],
    mut at: usize,
    mut from: usize,
    mut hits: u128,
) -> u128 {
    while at < candidates.len() && from < block.len() {
        let (value, posting) = (candidates[at], block[from]);
        hits |= u128::from(value == posting) << at;
        at += usize::from(value <= posting);
        from += usize::from(posting <= value);
    }
    hits
}

/// Eight candidates against eight postings at a time: every rotation of the postings is
/// compared with the candidates, and the side whose last element is the smaller advances (both
/// when they are equal). The candidates of the group in progress keep their matches in
/// `group` until the group is passed.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn merge_hits_avx2(candidates: &[u32], block: &[u32]) -> u128 {
    use std::arch::x86_64::*;
    let (mut at, mut from) = (0usize, 0usize);
    let (mut hits, mut group) = (0u128, 0u32);
    // SAFETY: the loop reads eight elements at `at` and at `from` only while both fit.
    unsafe {
        let rotate = _mm256_setr_epi32(1, 2, 3, 4, 5, 6, 7, 0);
        while at + 8 <= candidates.len() && from + 8 <= block.len() {
            let wanted = _mm256_loadu_si256(candidates.as_ptr().add(at).cast());
            let mut other = _mm256_loadu_si256(block.as_ptr().add(from).cast());
            let mut equal = _mm256_cmpeq_epi32(wanted, other);
            for _ in 1..8 {
                other = _mm256_permutevar8x32_epi32(other, rotate);
                equal = _mm256_or_si256(equal, _mm256_cmpeq_epi32(wanted, other));
            }
            group |= _mm256_movemask_ps(_mm256_castsi256_ps(equal)) as u32;
            let (most, top) = (candidates[at + 7], block[from + 7]);
            if most <= top {
                hits |= u128::from(group) << at;
                group = 0;
                at += 8;
            }
            if top <= most {
                from += 8;
            }
        }
    }
    // `group` is zero once every candidate has been passed.
    let partial = u128::from(group).checked_shl(at as u32).unwrap_or(0);
    merge_hits_scalar(candidates, block, at, from, hits | partial)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_hits_agree_with_a_direct_membership_test() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || crate::core::documents::xorshift64(&mut state);
        let shapes = [
            (8, 8, 8),
            (8, 8, 16),
            (9, 8, 40),
            (8, 9, 40),
            (31, 128, 200),
            (128, 128, 128),
            (128, 128, 300),
            (100, 17, 5000),
            (17, 100, 5000),
            (7, 100, 500),
            (128, 8, 1000),
            (1, 128, 300),
        ];
        for (a, b, span) in shapes {
            for _ in 0..100 {
                let mut draw = |count: usize| {
                    let mut set = std::collections::BTreeSet::new();
                    while set.len() < count {
                        set.insert((next() % span) as u32);
                    }
                    set.into_iter().collect::<Vec<u32>>()
                };
                let (candidates, block) = (draw(a), draw(b));
                let expected = candidates
                    .iter()
                    .enumerate()
                    .filter(|(_, slot)| block.binary_search(slot).is_ok())
                    .fold(0u128, |bits, (t, _)| bits | 1u128 << t);
                assert_eq!(merge_hits(&candidates, &block), expected);
                assert_eq!(merge_hits_scalar(&candidates, &block, 0, 0, 0), expected);
            }
        }
    }
}
