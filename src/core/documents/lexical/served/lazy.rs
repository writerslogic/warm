//! The conjunction's filter through a list whose frequencies it records.

use super::CHECKPOINT_BLOCK;
use crate::core::documents::lists::Cursor;

/// [`super::keep_common`] for a conjunction's frequencies: each kept candidate's frequency in `other`
/// goes to `rows[row]`, and a block's frequencies are decoded only when it holds a match.
pub(super) fn keep_common_lazy(
    other: &mut Cursor<'_>,
    slots: &mut [u32; CHECKPOINT_BLOCK],
    n: usize,
    rows: &mut [[u32; CHECKPOINT_BLOCK]],
    row: usize,
) -> (usize, bool) {
    let carried = row;
    let mut matched = [0u8; CHECKPOINT_BLOCK];
    let mut source = [0u8; CHECKPOINT_BLOCK];
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
        let from = k;
        let block = other.block_slots();
        // The candidates this block can hold: `slots[i..end]`.
        let mut end = i + 1;
        while end < n && slots[end] <= last {
            end += 1;
        }
        if (end - i) * 16 < block.len() {
            // Few candidates: a branch-free binary search each.
            let mut j = 0usize;
            let mut at = i;
            while at < end {
                let value = slots[at];
                let mut size = block.len() - j;
                while size > 1 {
                    let half = size / 2;
                    j += half * usize::from(block[j + half] <= value);
                    size -= half;
                }
                slots[k] = value;
                matched[k] = j as u8;
                source[k] = at as u8;
                k += usize::from(block[j] == value);
                at += 1;
            }
        } else {
            // Many: a branch-free merge, each step advancing whichever side is behind.
            let mut j = 0usize;
            let mut at = i;
            while at < end && j < block.len() {
                let (value, posting) = (slots[at], block[j]);
                slots[k] = value;
                matched[k] = j as u8;
                source[k] = at as u8;
                k += usize::from(value == posting);
                at += usize::from(value <= posting);
                j += usize::from(posting <= value);
            }
        }
        i = end;
        // The carried rows move after the block, a row at a time: moved per candidate across
        // the rows, the copy vectorizes into 512-bit gathers and scatters (see above).
        for row in rows[..carried].iter_mut() {
            for (m, &at) in source.iter().enumerate().take(k).skip(from) {
                row[m] = row[usize::from(at)];
            }
        }
        if k > from {
            // Only a block with a match has its frequencies decoded; `black_box` keeps the
            // copy scalar, as a gather's frequency license slows the queries that follow on
            // AVX-512 parts.
            let frequencies = other.block_postings().1;
            for (m, &j) in matched.iter().enumerate().take(k).skip(from) {
                rows[row][m] = frequencies[std::hint::black_box(usize::from(j))];
            }
        }
    }
    (k, true)
}
