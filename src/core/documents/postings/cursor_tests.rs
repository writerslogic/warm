// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

#[test]
fn next_and_advance_match_sorted_runs_at_block_and_integer_limits() {
    for length in [0, 1, 127, 128, 129, 255, 256, 257] {
        for gap in [1usize, 127, 128, 16_384] {
            let mut run: Vec<Posting> = (0..length)
                .map(|i| (i * gap, if i % 2 == 0 { 1 } else { u32::MAX }))
                .collect();
            run.push((usize::MAX, 0));
            let mut list = PostingList::default();
            for batch in run.chunks(17) {
                list.append_run(batch);
            }
            assert_eq!(list.iter().collect::<Vec<_>>(), run);
            assert_eq!(list.positive_len(), length);
            for target in [
                0,
                1,
                126 * gap,
                127 * gap,
                128 * gap,
                129 * gap,
                usize::MAX - 1,
                usize::MAX,
            ] {
                let mut cursor = list.iter();
                let expected = run.iter().copied().find(|p| p.0 >= target);
                assert_eq!(cursor.advance(target), expected);
                assert_eq!(cursor.advance(target), expected);
                assert_eq!(cursor.next(), expected);
                assert_eq!(
                    cursor.collect::<Vec<_>>(),
                    run.iter()
                        .copied()
                        .filter(|p| p.0 > expected.unwrap().0)
                        .collect::<Vec<_>>()
                );
            }
            let mut cursor = list.iter();
            assert_eq!(cursor.advance(usize::MAX), Some((usize::MAX, 0)));
            assert_eq!(cursor.next(), Some((usize::MAX, 0)));
            assert_eq!(cursor.advance(usize::MAX), None);
        }
    }
}
