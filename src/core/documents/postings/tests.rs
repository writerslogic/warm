// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

#[test]
fn parallel_install_matches_serial_install_list_for_list() {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let names: Vec<String> = (0..4000).map(|i| format!("term{i:05}")).collect();
    let word_counts: Vec<u32> = (0..4000).map(|i| 1 + (i % 37) as u32).collect();
    let (mut serial, mut parallel) = (PostingLists::default(), PostingLists::default());
    let mut next_slot = 0usize;
    // Early batches add new terms; later ones append to existing lists and add more.
    for batch in 0..6 {
        let slots: Vec<usize> = (0..300)
            .map(|_| {
                next_slot += 1 + (next() % 3) as usize;
                next_slot
            })
            .collect();
        let mut runs: Vec<TermRun<'_>> = Vec::new();
        for name in names.iter().take(1500 + 400 * batch) {
            if next() % 4 == 0 {
                continue;
            }
            let mut postings: Vec<Posting> = Vec::new();
            for &slot in &slots {
                if next() % 5 == 0 {
                    postings.push((slot, 1 + (next() % 9) as u32));
                }
            }
            if postings.is_empty() {
                continue;
            }
            let ordinals = postings.iter().map(|_| (next() % 70_000) as u32).collect();
            runs.push(TermRun {
                term: name,
                postings,
                ordinals,
            });
        }
        assert!(runs.len() >= PARALLEL_LOOKUPS);
        if batch > 0 {
            // Empty some lists so this batch's new terms reuse freed ids out of run order.
            for name in names.iter().step_by(7).take(150) {
                let Some(id) = serial.dictionary.get(name) else {
                    continue;
                };
                let mut slots = Vec::new();
                let mut cursor = serial.lists[id as usize].iter();
                while let Some((slot, _)) = cursor.peek() {
                    slots.push(slot);
                    cursor.next();
                }
                for slot in slots {
                    serial.remove(name, slot, &word_counts);
                    parallel.remove(name, slot, &word_counts);
                }
            }
            assert!(!serial.free.is_empty());
            assert_eq!(serial.free, parallel.free);
        }
        serial.install_runs(&runs, &word_counts, false);
        parallel.install_runs(&runs, &word_counts, true);
        parallel.assert_valid();
        assert_eq!(serial.lists.len(), parallel.lists.len());
        for (id, (a, b)) in serial.lists.iter().zip(&parallel.lists).enumerate() {
            let (mut x, mut y) = (Vec::new(), Vec::new());
            pack_checkpoint(a, &|_| 1, false, &mut x, None);
            pack_checkpoint(b, &|_| 1, false, &mut y, None);
            assert_eq!(x, y, "list {id} after batch {batch}");
            assert_eq!(a.block_count(), b.block_count());
            for block in 0..a.block_count() {
                assert_eq!(a.block_bounds(block), b.block_bounds(block));
            }
        }
        for name in &names {
            assert_eq!(serial.dictionary.get(name), parallel.dictionary.get(name));
        }
    }
}

#[test]
fn positional_ordinals_survive_block_edits_without_changing_checkpoint_bytes() {
    for count in [127, 128, 129] {
        let mut list = PostingList::default();
        let mut model = BTreeMap::new();
        let ordinals = [0, 1, 127, 128, 129, 255, 256, 257, 65_535, 65_536, 65_537];
        let check = |list: &PostingList, model: &BTreeMap<usize, (u32, u32)>| {
            let mut cursor = list.iter();
            for (&slot, &(frequency, ordinal)) in model {
                assert_eq!(cursor.peek(), Some((slot, frequency)));
                assert_eq!(cursor.ordinal(), ordinal);
                cursor.next();
            }
            assert!(cursor.peek().is_none());
            let mut plain = PostingList::default();
            plain.append_run(&model.iter().map(|(&s, &(f, _))| (s, f)).collect::<Vec<_>>());
            let (mut actual, mut expected) = (Vec::new(), Vec::new());
            pack_checkpoint(list, &|_| 1, true, &mut actual, None);
            pack_checkpoint(&plain, &|_| 1, true, &mut expected, None);
            assert_eq!(actual, expected);
        };
        for i in 0..count {
            let (slot, frequency, ordinal) =
                (i * 2, 1 + i as u32 % 3, ordinals[i % ordinals.len()]);
            list.append_run_with_ordinals(&[(slot, frequency)], &[ordinal], |_| 100);
            model.insert(slot, (frequency, ordinal));
        }
        check(&list, &model);
        for i in 0..100 {
            list.remove(i * 2, |_| 100);
            model.remove(&(i * 2));
        }
        check(&list, &model);
        list.append_run_with_ordinals(&[(1, 2)], &[255], |_| 100);
        model.insert(1, (2, 255));
        let run: Vec<_> = (0..100).map(|i| (i * 2, 3)).collect();
        let ordinals: Vec<_> = (0..100).map(|i| i * 1024 + 1).collect();
        list.append_run_with_ordinals(&run, &ordinals, |_| 100);
        for (&(slot, frequency), &ordinal) in run.iter().zip(&ordinals) {
            model.insert(slot, (frequency, ordinal));
        }
        check(&list, &model);
        for slot in [0, 1, 128, 200] {
            list.set_entry(slot, 4, Some(65_537), |_| 100);
            model.insert(slot, (4, 65_537));
        }
        check(&list, &model);
    }
}

/// An edit of one posting in a long list recomputes one block's frontier (two when the
/// block splits), where re-encoding the list computes one per block.
#[test]
fn single_posting_edits_recompute_one_block_frontier() {
    let blocks = 64;
    let run: Vec<Posting> = (0..blocks * BLOCK_LEN)
        .map(|i| (i * 2, 1 + (i % 7) as u32))
        .collect();
    let words = |slot: usize| 10 + (slot % 13) as u32;
    let mut list = PostingList::default();
    list.append_run_with_lengths(&run, words);
    for name in ["remove", "insert", "reweight"] {
        let before = frontiers_computed();
        match name {
            "remove" => list.remove(4000, words),
            "insert" => list.append_run_with_lengths(&[(4000, 3)], words),
            _ => list.set_frequency(4000, 9, words),
        }
        assert!(frontiers_computed() - before <= 2, "{name}");
    }
    let before = frontiers_computed();
    list.encode(&list.decode(), words);
    assert_eq!(frontiers_computed() - before, list.block_count());
    assert!(list.block_count() >= blocks);
}

#[test]
fn gallop_matches_partition_point() {
    for length in 0..70usize {
        let items: Vec<usize> = (0..length).map(|i| i * 3).collect();
        for target in 0..=length * 3 + 1 {
            assert_eq!(
                gallop(&items, |&x| x < target),
                items.partition_point(|&x| x < target)
            );
        }
    }
}

/// The constant-width and general unpackers agree with single-value reads at every
/// width, count and trailing slack.
#[test]
fn block_unpacking_matches_single_reads_at_every_width() {
    for bits in 0..=64u8 {
        for count in [0, 1, 7, 8, 9, 31, 32, 33, 63, 64, 65, 127, BLOCK_LEN] {
            let limit = if bits == 64 {
                u64::MAX
            } else {
                (1u64 << bits) - 1
            };
            let values: Vec<u64> = (0..count as u64)
                .map(|i| i.wrapping_mul(0x9E37_79B9_7F4A_7C15) & limit)
                .collect();
            for slack in [0usize, 3, 7, 8, 9] {
                let mut bytes = Vec::new();
                pack(&mut bytes, values.iter().copied(), bits);
                bytes.extend(std::iter::repeat_n(0xFF, slack));
                let mut out = [0u64; BLOCK_LEN];
                unpack_values(&bytes, count, bits, &mut out);
                assert_eq!(
                    &out[..count],
                    values.as_slice(),
                    "bits={bits} count={count}"
                );
                for (index, &value) in values.iter().enumerate() {
                    assert_eq!(unpack(&bytes, index, bits), value);
                }
            }
        }
    }
}

#[test]
fn block_unpacking_preserves_output_after_short_and_unaligned_blocks() {
    for bits in 1..=32u8 {
        let mask = (1u64 << bits) - 1;
        for count in [31, 32, 33, 127, 128] {
            let values: Vec<u64> = (0..count)
                .map(|i| match i % 3 {
                    0 => mask,
                    1 => 0,
                    _ => (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) & mask,
                })
                .collect();
            for offset in 0..32 {
                for slack in [0, 3, 4, 5, 7, 8, 15, 16] {
                    let mut bytes = vec![0xcc; offset];
                    pack(&mut bytes, values.iter().copied(), bits);
                    bytes.extend(std::iter::repeat_n(0xff, slack));
                    let mut out = [u64::MAX; BLOCK_LEN];
                    unpack_values(&bytes[offset..], count, bits, &mut out);
                    assert_eq!(&out[..count], values, "bits={bits}, count={count}");
                    assert!(out[count..].iter().all(|&value| value == u64::MAX));
                }
            }
        }
    }
}

/// `list` packed in the block (`blocked`) or flat checkpoint encoding, decoded by the fast
/// and the checked decoder, which must agree.
fn decode_both(list: &PostingList, blocked: bool) -> Vec<Posting> {
    let words = |slot: usize| (slot % 7) as u32 + 1;
    let mut result = Vec::new();
    for impacts in [false, true] {
        if impacts && !blocked {
            continue;
        }
        let mut bytes = Vec::new();
        if blocked {
            pack_checkpoint(list, &words, impacts, &mut bytes, None);
        } else {
            pack_checkpoint_flat(list, &mut bytes);
        }
        let packed = Postings {
            bytes: &bytes,
            count: list.len() as u32,
            blocked,
            impacts,
        };
        assert!(checkpoint_min_len(list.len() as u64, blocked, impacts) <= bytes.len() as u64);
        let mut fast = Vec::new();
        packed.for_each(|slot, frequency| fast.push((slot, frequency)));
        let mut checked = Vec::new();
        packed
            .try_for_each(&words, |slot, frequency| {
                checked.push((slot, frequency));
                true
            })
            .unwrap();
        assert_eq!(fast, checked, "blocked {blocked} impacts {impacts}");
        // A list cut short or extended by a byte fails the checked decoder.
        for broken in [
            &bytes[..bytes.len() - 1],
            &[bytes.as_slice(), &[0]].concat()[..],
        ] {
            let broken = Postings {
                bytes: broken,
                ..packed
            };
            assert!(
                broken.try_for_each(&words, |_, _| true).is_err(),
                "blocked {blocked} impacts {impacts}"
            );
        }
        if impacts {
            // A recorded bound that the postings do not meet fails it.
            let mut lied = bytes.clone();
            lied[1] = lied[1].wrapping_add(1);
            let lied = Postings {
                bytes: &lied,
                ..packed
            };
            assert!(lied.try_for_each(&words, |_, _| true).is_err());
        }
        result = checked;
    }
    result
}

/// The packed checkpoint decoder agrees with the checked one for every mix of one-byte
/// postings, wide gaps and frequencies at every position in the eight-byte window.
#[test]
fn checkpoint_fast_decoding_matches_checked_decoding() {
    for pattern in 0u32..256 {
        let mut postings = Vec::new();
        let mut slot = 0usize;
        for i in 0..67u32 {
            let wide = (pattern >> (i % 8)) & 1 == 1;
            slot += if wide {
                70 + (i as usize) * 31
            } else {
                1 + (i as usize) % 60
            };
            let frequency = if i % 11 == 3 { 2 + i } else { 1 };
            postings.push((slot, frequency));
        }
        let mut list = PostingList::default();
        list.append_run(&postings);
        for blocked in [false, true] {
            assert_eq!(decode_both(&list, blocked), postings, "pattern {pattern}");
        }
    }
}

#[test]
fn checkpoint_decoding_matches_at_every_varint_length_and_window_offset() {
    // Word varints change length at gaps 64 and 8,192; frequency varints at 128 and 16,384.
    let gaps = [1usize, 63, 64, 65, 8_191, 8_192, 8_193, 1 << 20];
    let frequencies = [1u32, 2, 127, 128, 129, 16_383, 16_384, 16_385, u32::MAX];
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    for length in [
        1usize, 7, 8, 9, 15, 16, 17, 64, 127, 128, 129, 256, 257, 1_000,
    ] {
        for _ in 0..40 {
            let mut slot = 0usize;
            let postings: Vec<Posting> = (0..length)
                .map(|i| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    // Mostly short postings, so runs of them straddle each window offset.
                    let short = !state.is_multiple_of(4);
                    let gap = if short {
                        1 + (state >> 8) as usize % 60
                    } else {
                        gaps[(state >> 8) as usize % gaps.len()]
                    };
                    let frequency = if short && state & 0x30 != 0 {
                        1
                    } else {
                        frequencies[(state >> 24) as usize % frequencies.len()]
                    };
                    slot += if i == 0 { gap - 1 } else { gap };
                    (slot, frequency)
                })
                .collect();
            let mut list = PostingList::default();
            list.append_run(&postings);
            for blocked in [false, true] {
                assert_eq!(decode_both(&list, blocked), postings, "{length} {blocked}");
            }
        }
    }
}

#[test]
fn initial_block_capacity_matches_the_first_run() {
    for length in [0, 1, 127, 128, 129, 255, 256, 257, 511, 512, 513] {
        let run: Vec<Posting> = (0..length).map(|slot| (slot, 1)).collect();
        let mut list = PostingList::default();
        list.append_run(&run);
        assert_eq!(list.blocks.capacity(), length.div_ceil(BLOCK_LEN));
        assert_eq!(list.decode(), run);
        list.append_run(&[(length, 2)]);
        assert_eq!(list.len(), length + 1);
        assert_eq!(list.iter().last(), Some((length, 2)));
    }
}

#[test]
fn frontiers_match_pairwise_dominance() {
    let check = |points: &[(u32, u32)]| {
        let mut expected: Vec<_> = points
            .iter()
            .copied()
            .filter(|&(frequency, words)| {
                frequency > 0
                    && !points.iter().any(|&(other_frequency, other_words)| {
                        other_frequency >= frequency
                            && other_words <= words
                            && (other_frequency > frequency || other_words < words)
                    })
            })
            .collect();
        expected.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        expected.dedup();
        let expected = (expected.len() > 1).then_some(expected);
        assert_eq!(
            frontier(points.iter().copied()).as_deref(),
            expected.as_deref()
        );
    };
    for points in [
        vec![],
        vec![(0, 0), (0, u32::MAX)],
        vec![(1, u32::MAX)],
        vec![(1, 0), (1, 0), (u32::MAX, u32::MAX)],
        vec![(0, 0), (2, 5), (2, 7), (1, 5), (3, 8), (1, 1)],
    ] {
        check(&points);
    }
    for length in [BLOCK_LEN - 1, BLOCK_LEN] {
        check(
            &(0..length)
                .map(|i| (i as u32 + 1, i as u32))
                .collect::<Vec<_>>(),
        );
        for seed in 0..64 {
            let points: Vec<_> = (0..length)
                .map(|i| (((i * 37 + seed) % 13) as u32, ((i * 17 + seed) % 23) as u32))
                .collect();
            check(&points);
        }
    }
}

#[test]
#[should_panic(expected = "frontier exceeds posting block length")]
fn frontiers_reject_more_points_than_a_block_can_hold() {
    frontier(std::iter::repeat_n((1, 1), BLOCK_LEN + 1));
}

#[test]
fn partial_blocks_preserve_full_frontiers_and_invalidated_fallbacks() {
    let run: Vec<_> = (0..BLOCK_LEN).map(|i| (i, i as u32 + 1)).collect();
    let mut list = PostingList::default();
    for posting in &run {
        list.append_run_with_lengths(std::slice::from_ref(posting), |slot| slot as u32 + 1);
    }
    let expected: Vec<_> = run
        .iter()
        .map(|&(slot, frequency)| (frequency, slot as u32 + 1))
        .rev()
        .collect();
    assert_eq!(
        list.blocks[0].frontier.as_deref(),
        Some(expected.as_slice())
    );
    assert_eq!(list.decode(), run);

    let mut list = PostingList::default();
    list.append_run_with_lengths(&[(0, 5)], |_| 10);
    list.invalidate_length(0);
    list.append_run_with_lengths(&[(1, 6), (2, 0)], |_| 20);
    assert_eq!(
        list.blocks[0].frontier.as_deref(),
        Some([(6, 20), (5, 0)].as_slice())
    );
    assert_eq!(list.blocks[0].min_words, 0);
    assert_eq!(list.blocks[0].max_frequency, 6);
}

/// Insert, update and remove against a map model over the full slot and frequency
/// ranges, with every lookup checked through a rebuilt dictionary too.
#[test]
fn posting_lists_round_trip_against_a_map_model() {
    let mut lists = PostingLists::default();
    let mut model: BTreeMap<String, BTreeMap<usize, u32>> = BTreeMap::new();
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let slots = [
        0usize,
        1,
        127,
        128,
        16_383,
        16_384,
        u32::MAX as usize,
        usize::MAX - 1,
        usize::MAX,
    ];
    let frequencies = [0u32, 1, 127, 128, u32::MAX - 1, u32::MAX];
    let check = |lists: &PostingLists, model: &BTreeMap<String, BTreeMap<usize, u32>>| {
        lists.assert_valid();
        assert_eq!(lists.term_count(), model.len());
        for (term, postings) in model {
            let list = lists.get(term).expect("term present");
            assert_eq!(list.len(), postings.len());
            assert_eq!(
                list.iter().collect::<Vec<_>>(),
                postings.iter().map(|(&s, &f)| (s, f)).collect::<Vec<_>>()
            );
        }
        assert!(lists.get("absent").is_none());
    };
    for round in 0..4000 {
        let term = format!("t{}", next() % 300);
        let slot = if next() % 4 == 0 {
            slots[(next() % slots.len() as u64) as usize]
        } else {
            (next() % 5000) as usize
        };
        let frequency = frequencies[(next() % frequencies.len() as u64) as usize];
        let entry = model.entry(term.clone()).or_default();
        match next() % 3 {
            0 if entry.contains_key(&slot) => {
                lists.set_frequency(&term, slot, frequency, &[]);
                entry.insert(slot, frequency);
            }
            1 if entry.contains_key(&slot) => {
                lists.remove(&term, slot, &[]);
                entry.remove(&slot);
                if entry.is_empty() {
                    model.remove(&term);
                }
            }
            _ if !entry.contains_key(&slot) => {
                lists.insert(&term, slot, frequency, &[]);
                entry.insert(slot, frequency);
            }
            _ => {
                if entry.is_empty() {
                    model.remove(&term);
                }
            }
        }
        if round % 500 == 499 {
            lists.dictionary.rebuild();
            check(&lists, &model);
        }
    }
    check(&lists, &model);
}

/// Runs appended in bulk (above the final slot, and below it after removals) reach the
/// lists single inserts build, through both the overlay and the bulk dictionary merge.
#[test]
fn bulk_runs_match_single_inserts() {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let term_names: Vec<String> = (0..6000).map(|i| format!("term{i:05}")).collect();
    let mut bulk = PostingLists::default();
    let mut single = PostingLists::default();
    let mut next_slot = 0usize;
    // Slots a batch freed from every list, offered to the next batch for reuse.
    let mut freed: Vec<usize> = Vec::new();
    for batch in 0..6 {
        // Batches 0 and 3 are large (bulk dictionary merge), the others small (overlay).
        let batch_terms = if batch % 3 == 0 { 3000 } else { 40 };
        let mut runs: Vec<TermRun<'_>> = Vec::new();
        let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        let mut slots: Vec<usize> = std::mem::take(&mut freed);
        for _ in 0..200 {
            slots.push(next_slot);
            next_slot += 1 + (next() % 3) as usize;
        }
        slots.sort_unstable();
        for t in 0..batch_terms {
            let term = term_names[(next() % 6000) as usize % (t + 1)].as_str();
            if !seen.insert(term) {
                continue;
            }
            let mut postings: Vec<Posting> = Vec::new();
            for &slot in &slots {
                if next() % 3 == 0 {
                    postings.push((slot, 1 + (next() % 7) as u32));
                }
            }
            if postings.is_empty() {
                continue;
            }
            runs.push(TermRun {
                term,
                postings,
                ordinals: Vec::new(),
            });
        }
        for run in &runs {
            for &(slot, frequency) in &run.postings {
                single.insert(run.term, slot, frequency, &[]);
            }
        }
        bulk.insert_runs(&runs);
        // Free a tenth of this batch's slots from every list holding them (only this
        // batch's runs do), so the next batch's runs start below some final slots.
        freed = slots.iter().copied().filter(|_| next() % 10 == 0).collect();
        for run in &runs {
            for &(slot, _) in &run.postings {
                if freed.contains(&slot) {
                    bulk.remove(run.term, slot, &[]);
                    single.remove(run.term, slot, &[]);
                }
            }
        }
        bulk.assert_valid();
        single.assert_valid();
        assert_eq!(bulk.term_count(), single.term_count());
        for term in &term_names {
            let a = bulk.get(term).map(|l| l.iter().collect::<Vec<_>>());
            let b = single.get(term).map(|l| l.iter().collect::<Vec<_>>());
            assert_eq!(a, b, "{term} after batch {batch}");
        }
    }
}

#[test]
fn dictionary_lookups_survive_overlay_base_and_tombstones() {
    let mut d = TermDictionary::default();
    for i in 0..3000u32 {
        d.insert(&format!("w{i:05}"), i);
    }
    assert!(d.overlay.len() < 3000, "overlay folded into the base");
    for i in 0..3000u32 {
        assert_eq!(d.get(&format!("w{i:05}")), Some(i));
    }
    assert_eq!(d.get("w"), None);
    assert_eq!(d.get("w03000"), None);
    assert_eq!(d.get("w0150"), None);
    for i in (0..3000u32).step_by(2) {
        assert_eq!(d.remove(&format!("w{i:05}")), Some(i));
    }
    assert_eq!(d.len(), 1500);
    assert_eq!(d.remove("w00000"), None);
    assert_eq!(d.get("w00000"), None);
    assert_eq!(d.get("w00001"), Some(1));
    d.insert("w00000", 9);
    assert_eq!(d.get("w00000"), Some(9));
    d.rebuild();
    assert_eq!(d.len(), 1501);
    assert_eq!(d.get("w00000"), Some(9));
    assert_eq!(d.get("w02999"), Some(2999));
    assert_eq!(d.get("w02998"), None);
}

/// A bulk insert revives tombstoned base terms, keeps the overlay, and lands every term
/// in the base in sorted order.
#[test]
fn bulk_dictionary_merge_keeps_overlay_and_revives_tombstones() {
    let mut d = TermDictionary::default();
    for i in 0..2000u32 {
        d.insert(&format!("b{i:05}"), i);
    }
    d.rebuild();
    for i in (0..2000u32).step_by(3) {
        assert_eq!(d.remove(&format!("b{i:05}")), Some(i));
    }
    d.insert("a00001", 5000);
    d.insert("c00001", 5001);
    // 667 of these revive tombstones, the rest are new; together they exceed the overlay
    // budget (2000 / 4 + 1024), so the batch merges into the base.
    let names: Vec<String> = (0..2000u32).map(|i| format!("b{:05}", i * 3)).collect();
    let mut batch: Vec<(&str, u32)> = names
        .iter()
        .enumerate()
        .map(|(i, name)| (name.as_str(), 6000 + i as u32))
        .collect();
    batch.reverse();
    d.insert_many(batch);
    assert!(d.overlay.is_empty(), "bulk merge folds the overlay");
    assert_eq!(d.tombstones, 0);
    assert_eq!(d.get("a00001"), Some(5000));
    assert_eq!(d.get("c00001"), Some(5001));
    for (i, name) in names.iter().enumerate() {
        assert_eq!(d.get(name), Some(6000 + i as u32), "{name}");
    }
    assert_eq!(d.get("b00001"), Some(1));
    assert_eq!(d.get("b00002"), Some(2));
    assert_eq!(d.len(), 2000 - 667 + 2 + 2000);
    let terms: Vec<&[u8]> = (0..d.base_ids.len()).map(|i| d.base_term(i)).collect();
    assert!(terms.windows(2).all(|pair| pair[0] < pair[1]));
}

/// `for_each` (four one-byte postings per load where it can) visits exactly what `iter`
/// decodes, for lists of every length up to past two loads, with a multi-byte gap or
/// frequency at every position.
#[test]
fn for_each_matches_iter_across_packed_widths() {
    for length in 0..=13usize {
        for wide in 0..=length {
            for wide_frequency in [false, true] {
                let mut run = Vec::new();
                let mut slot = 5usize;
                for i in 0..length {
                    slot += if i == wide && !wide_frequency {
                        300
                    } else {
                        1 + i % 3
                    };
                    let frequency = if i == wide && wide_frequency {
                        70_000
                    } else {
                        1 + i as u32 % 127
                    };
                    run.push((slot, frequency));
                }
                let mut list = PostingList::default();
                list.append_run(&run);
                let mut visited = Vec::new();
                list.for_each(|slot, frequency| visited.push((slot, frequency)));
                assert_eq!(visited, list.iter().collect::<Vec<_>>());
                assert_eq!(visited, run, "length {length} wide {wide}");
            }
        }
    }
}
