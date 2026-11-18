// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;
use serde_json::json;

/// `chunk_spans` without the single-chunk ASCII count: every word's span collected, then
/// cut. The oracle for `chunk_spans_match_the_collecting_reference`.
fn chunk_spans_reference(input: &DocumentInput) -> Result<Vec<(u32, u32)>> {
    validate_document_source(input)?;
    let size = input.chunk_words.unwrap_or(256) as usize;
    let overlap = input
        .overlap_words
        .unwrap_or(32.min(size.saturating_sub(1)) as u32) as usize;
    ensure!(
        (1..=4096).contains(&size) && overlap < size,
        "chunkWords must be 1..=4096 and overlapWords smaller than chunkWords"
    );
    let mut words = Vec::new();
    let mut start = None;
    let mut boundary = |i: usize, whitespace: bool| {
        if whitespace {
            if let Some(s) = start.take() {
                words.push((s, i));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    };
    if input.text.is_ascii() {
        for (i, b) in input.text.bytes().enumerate() {
            boundary(i, matches!(b, b' ' | b'\t'..=b'\r'));
        }
    } else {
        for (i, c) in input.text.char_indices() {
            boundary(i, c.is_whitespace());
        }
    }
    if let Some(s) = start {
        words.push((s, input.text.len()));
    }
    ensure!(!words.is_empty(), "document must contain text");
    let mut chunks = Vec::new();
    let mut word = 0;
    while word < words.len() {
        ensure!(
            chunks.len() < 4096,
            "document exceeds 4096 chunks; increase chunkWords"
        );
        let end_word = (word + size).min(words.len());
        let (start_byte, end_byte) = (words[word].0, words[end_word - 1].1);
        ensure!(
            end_byte - start_byte <= MAX_CHUNK_BYTES,
            "chunk exceeds 64 KiB; reduce chunkWords or split long tokens"
        );
        chunks.push((start_byte as u32, end_byte as u32));
        if end_word == words.len() {
            break;
        }
        word += size - overlap;
    }
    Ok(chunks)
}

/// The single-chunk scan cuts every text exactly as collecting all word spans does, with
/// the same errors: word counts at the chunk size -1/0/+1, chunk bytes at 64 KiB -1/0/+1,
/// the 4096-chunk limit -1/0/+1, Unicode and ASCII whitespace, overlap, and no words.
#[test]
fn chunk_spans_match_the_collecting_reference() {
    let words = |n: usize, word: &str, gap: &str| vec![word; n].join(gap);
    let mut cases: Vec<(String, Option<u32>, Option<u32>)> = Vec::new();
    for gap in [
        " ",
        "\t\n ",
        "\u{a0}",
        "\u{85}",
        "\u{3000}",
        "\u{2028}x\u{1c}",
    ] {
        for size in [1u32, 2, 3, 8, 256] {
            for n in [
                0,
                1,
                2,
                size as usize - 1,
                size as usize,
                size as usize + 1,
                3 * size as usize + 1,
            ] {
                for overlap in [None, Some(0), Some(size - 1)] {
                    cases.push((format!(" {} ", words(n, "wörd", gap)), Some(size), overlap));
                    cases.push((words(n, "w", gap), Some(size), overlap));
                }
            }
        }
    }
    // One chunk's byte span at 64 KiB - 1, 64 KiB and 64 KiB + 1: one long word.
    for len in [MAX_CHUNK_BYTES - 1, MAX_CHUNK_BYTES, MAX_CHUNK_BYTES + 1] {
        cases.push((format!("  {}\n", "a".repeat(len)), None, None));
        cases.push((format!("a {} b", "c".repeat(len - 4)), Some(4096), Some(0)));
        cases.push((format!("a {} b", "c".repeat(len - 4)), Some(2), Some(1)));
    }
    // 4095, 4096 and 4097 chunks of one word each.
    for n in [4095, 4096, 4097] {
        cases.push((words(n, "w", " "), Some(1), Some(0)));
    }
    cases.push((String::new(), None, None));
    cases.push((" \t\u{3000}\u{a0} ".into(), None, None));
    cases.push(("one".into(), Some(0), None));
    cases.push(("one".into(), Some(2), Some(2)));
    for (text, chunk_words, overlap_words) in cases {
        let input: DocumentInput = serde_json::from_value(json!({
            "id": "d", "text": text, "chunkWords": chunk_words, "overlapWords": overlap_words,
        }))
        .unwrap();
        let got = chunk_spans(&input).map_err(|e| e.to_string());
        let want = chunk_spans_reference(&input).map_err(|e| e.to_string());
        assert_eq!(
            got,
            want,
            "{chunk_words:?} {overlap_words:?} {:?}",
            input.text.chars().take(40).collect::<String>()
        );
    }
}

/// Equal metadata shares one allocation, unequal metadata does not, the stored form is
/// the value's own JSON, and a write through one sharer is not seen by the other.
#[test]
fn interned_metadata_shares_equal_values_only() {
    let mut seen = std::collections::HashMap::new();
    let mut first: SharedMetadata = json!({"bucket": 3, "tags": ["a", 1.5]}).into();
    let mut equal: SharedMetadata = json!({"tags": ["a", 1.5], "bucket": 3}).into();
    let mut other: SharedMetadata = json!({"bucket": 4, "tags": ["a", 1.5]}).into();
    for metadata in [&mut first, &mut equal, &mut other] {
        metadata.intern(&mut seen);
    }
    assert!(Arc::ptr_eq(&first.0, &equal.0));
    assert!(!Arc::ptr_eq(&first.0, &other.0));
    assert_eq!(
        serde_json::to_string(&equal).unwrap(),
        serde_json::to_string(&json!({"bucket": 3, "tags": ["a", 1.5]})).unwrap()
    );
    equal["bucket"] = json!(9);
    assert_eq!(first["bucket"], 3);
    assert_eq!(equal["bucket"], 9);
}

/// The chunk id keeps the journal's text form, and recognising it agrees with comparing
/// against it, near misses included.
#[test]
fn chunk_ids_match_their_text_form() {
    for document in ["d", "a:b", "x:1:2", "ünï", ""] {
        for index in [0, 9, 10, 99, 100, 4095, usize::MAX] {
            let id = chunk_id(document, index);
            let text = format!("{CHUNK_PREFIX}{}:{document}:{index}", document.len());
            assert_eq!(id, text);
            assert_eq!(id.capacity(), id.len());
            assert!(is_chunk_id(&id, document, index));
            for other in [
                format!("{id}0"),
                id[..id.len() - 1].to_owned(),
                chunk_id(document, index.wrapping_add(1)),
                chunk_id(&format!("{document}x"), index),
                id.replacen(CHUNK_PREFIX, "", 1),
            ] {
                assert_eq!(
                    is_chunk_id(&other, document, index),
                    other == id,
                    "{other:?}"
                );
            }
        }
    }
}

fn document(i: usize, revision: usize) -> StoredDocument {
    let input: DocumentInput = serde_json::from_value(json!({
        "id":format!("d{i:03}"), "version":revision.to_string(),
        "text":format!("running common topic{} rare{i} shared birds fly distant place revision{revision}",i%7),
        "sourceUri":format!("source{}",i%3), "metadata":{"group":i%5,"empty":null,"flag":i.is_multiple_of(2),"text":"n","fraction":-0.0,"nested":{"key":i%5}},
        "chunkWords":5,"overlapWords":1,
        "embeddings":[[1.0,0.1,0.0],[0.0,1.0,0.1],[0.1,0.0,1.0]]
    })).unwrap();
    let mut doc = prepare(input, 1024, Some(3), TextEncoder::default()).unwrap();
    if i.is_multiple_of(2) {
        for chunk in &mut doc.chunks {
            chunk.terms = terms(chunk.text.as_deref().unwrap(), false).into();
            chunk.stemmed = false;
        }
    }
    doc
}

#[test]
fn bm25_options_change_scores_only_when_set() {
    let mut store = DocumentStore::default();
    for i in 0..12 {
        let doc = document(i, 1);
        store.insert(doc.id.clone(), doc);
    }
    let lexical = |k1: Option<f64>, b: Option<f64>| {
        let options = SearchOptions {
            k: Some(5),
            lexical_weight: Some(1.0),
            semantic_weight: Some(0.0),
            bm25_k1: k1,
            bm25_b: b,
            ..Default::default()
        };
        search_impl(&store, "rare3 shared", &options, Some(3), true).map(|hits| {
            hits.into_iter()
                .map(|h| (h.id, h.lexical_score))
                .collect::<Vec<_>>()
        })
    };
    let default = lexical(None, None).unwrap();
    assert_eq!(default, lexical(Some(1.2), Some(0.75)).unwrap());
    // k1 = 0 saturates every term at its idf, so the top hit scores the sum of its idfs.
    let flat = lexical(Some(0.0), Some(0.5)).unwrap();
    assert_ne!(flat, default);
    let n = store.chunk_count as f64;
    let idf = |word: &str| {
        let df = store.postings[1].get(word).map_or(0, |list| list.len()) as f64;
        (1.0 + (n - df + 0.5) / (df + 0.5)).ln()
    };
    let stem = |word: &str| terms(word, true).into_keys().next().unwrap();
    assert!((flat[0].1 - (idf(&stem("rare3")) + idf(&stem("shared")))).abs() < 1e-12);
    assert!(lexical(Some(-1.0), None).is_err());
    assert!(lexical(None, Some(1.5)).is_err());
    assert!(lexical(Some(f64::NAN), None).is_err());
    assert_scan_parity(&store);
}

#[test]
fn extreme_bm25_parameters_keep_all_search_scores_finite() {
    let mut store = DocumentStore::default();
    for (id, text) in [
        ("repeat", "red red red red"),
        ("single", "red"),
        ("other", "blue green"),
    ] {
        let input = serde_json::from_value(json!({
            "id": id,
            "text": text,
            "chunkWords": 16,
            "overlapWords": 0,
            "embeddings": [[1.0, 0.0, 0.0]],
        }))
        .unwrap();
        let document = prepare(input, 1024, Some(3), TextEncoder::default()).unwrap();
        store.insert(id.to_owned(), document);
    }
    let finite = |value: f64, b: f64| {
        let options = SearchOptions {
            k: Some(3),
            lexical_weight: Some(1.0),
            semantic_weight: Some(0.0),
            bm25_k1: Some(value),
            bm25_b: Some(b),
            ..Default::default()
        };
        let ordinary = search_impl(&store, "red", &options, Some(3), false).unwrap();
        let indexed = search_impl(&store, "red", &options, Some(3), true).unwrap();
        assert_eq!(
            serde_json::to_value(&ordinary).unwrap(),
            serde_json::to_value(&indexed).unwrap()
        );
        for hit in &ordinary {
            assert!(hit.score.is_finite());
            assert!(hit.lexical_score.is_finite());
            assert!(hit.semantic_score.is_none_or(f64::is_finite));
        }
        let lexical = lexical::execute(
            &store,
            &lexical::LexicalQuery::parse("red").unwrap(),
            &lexical::LexicalOptions {
                k: 3,
                bm25_k1: value,
                bm25_b: b,
                execution: lexical::LexicalExecution::Exhaustive,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            lexical
                .hits
                .iter()
                .map(|hit| &hit.document_id)
                .collect::<Vec<_>>(),
            ordinary
                .iter()
                .map(|hit| &hit.document_id)
                .collect::<Vec<_>>()
        );
        assert!(lexical.hits.iter().all(|hit| hit.score.is_finite()));
        for execution in [
            lexical::LexicalExecution::Auto,
            lexical::LexicalExecution::BlockMaxWand,
        ] {
            let optimized = lexical::execute(
                &store,
                &lexical::LexicalQuery::parse("red").unwrap(),
                &lexical::LexicalOptions {
                    k: 3,
                    bm25_k1: value,
                    bm25_b: b,
                    execution,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(optimized, lexical, "k1 {value}, b {b}");
        }
        serde_json::to_string(&ordinary).unwrap();
        serde_json::to_string(&lexical).unwrap();
    };
    for k1 in [
        0.0,
        f64::from_bits(1),
        f64::MIN_POSITIVE,
        f64::from_bits(1.0f64.to_bits() - 1),
        1.0,
        f64::from_bits(1.0f64.to_bits() + 1),
        f64::from_bits(f64::MAX.to_bits() - 1),
        f64::MAX,
    ] {
        for b in [0.0, 0.75, 1.0] {
            finite(k1, b);
        }
    }
    assert!(Bm25::from_options(&SearchOptions {
        bm25_k1: Some(f64::INFINITY),
        ..Default::default()
    })
    .is_err());
}

fn assert_scan_parity(store: &DocumentStore) {
    for query in [
        "running rare3 missing absent unknown extra terms for selective postings",
        "running rare3",
        "birds distant",
        "common",
        "missing token",
        "",
    ] {
        for fusion in ["blend", "rrf"] {
            for filter in [
                None,
                Some(json!({"group":3})),
                Some(json!({"group":99})),
                Some(json!({"empty":null,"flag":true,"text":"n"})),
                Some(json!({"fraction":0.0,"nested":{"key":3}})),
            ] {
                for limit in [9, 10, 11, 100] {
                    for embedding in [None, Some(vec![1., 0., 0.])] {
                        let opts = SearchOptions {
                            k: Some(9),
                            candidate_limit: Some(limit),
                            filter: filter.clone(),
                            fusion: Some(fusion.into()),
                            embedding,
                            min_semantic_score: Some(-1.),
                            ..Default::default()
                        };
                        let indexed = search_impl(store, query, &opts, Some(3), true).unwrap();
                        let scanned = search_impl(store, query, &opts, Some(3), false).unwrap();
                        assert_eq!(
                            serde_json::to_value(indexed).unwrap(),
                            serde_json::to_value(scanned).unwrap()
                        );
                    }
                }
            }
        }
    }
    let opts = SearchOptions {
        source_uri: Some("source1".into()),
        document_ids: Some(vec![
            "d010".into(),
            "d003".into(),
            "d010".into(),
            "absent".into(),
        ]),
        ..Default::default()
    };
    assert_eq!(
        serde_json::to_value(search_impl(store, "running", &opts, Some(3), true).unwrap()).unwrap(),
        serde_json::to_value(search_impl(store, "running", &opts, Some(3), false).unwrap())
            .unwrap()
    );
}

#[test]
fn postings_preserve_filtered_mixed_tokenization_scores_after_slot_reuse() {
    let mut store = DocumentStore::default();
    for i in 0..40 {
        let doc = document(i, 1);
        store.insert(doc.id.clone(), doc);
    }
    let query = "running rare3 missing absent unknown extra terms for selective postings";
    let forms = [query_words(query, false), query_words(query, true)];
    let posting_work: usize = forms
        .iter()
        .enumerate()
        .map(|(form, terms)| {
            terms
                .iter()
                .map(|word| store.postings[form].get(word).map_or(0, |p| p.len()))
                .sum::<usize>()
        })
        .sum();
    let eligible = store
        .values()
        .filter(|d| d.metadata["group"] == 3)
        .map(|d| d.chunks.len())
        .sum::<usize>();
    assert!(
        posting_work + store.slot_count
            < eligible
                * forms
                    .iter()
                    .map(std::collections::BTreeSet::len)
                    .sum::<usize>()
    );
    assert_scan_parity(&store);
    for i in (0..40).step_by(2) {
        store.remove(&format!("d{i:03}"));
    }
    for i in 0..20 {
        let mut doc = document(i, 2);
        if i == 3 {
            doc.metadata["group"] = json!(22);
            doc.source_uri = Some("replacement-source".into());
        }
        store.insert(doc.id.clone(), doc);
    }
    assert_scan_parity(&store);
    let ids: Vec<_> = store.values().map(|doc| doc.id.clone()).collect();
    for id in ids {
        store.remove(&id);
    }
    assert_eq!(store.slot_count, 0);
    assert!(store.postings.iter().all(|p| p.term_count() == 0));
    let doc = document(0, 3);
    store.insert(doc.id.clone(), doc);
    assert_scan_parity(&store);
}

#[test]
fn deleting_low_slots_reclaims_postings_with_high_slot_survivors() {
    for n in [1023, 1024, 1025] {
        let mut store = DocumentStore::default();
        for i in 0..n {
            let mut doc = document(i, 1);
            doc.chunks.truncate(1);
            store.insert(doc.id.clone(), doc);
        }
        for i in 0..n - 1 {
            store.remove(&format!("d{i:03}"));
        }
        assert!(store.slot_count <= 1024);
        assert_eq!(store.values().count(), 1);
        for form in &store.postings {
            form.assert_valid();
        }
        assert_scan_parity(&store);
    }
}

#[test]
fn restored_zero_frequency_terms_do_not_become_lexical_candidates() {
    let mut store = DocumentStore::default();
    let mut document = document(0, 1);
    for chunk in &mut document.chunks {
        chunk.terms.insert("ghost".into(), 0);
    }
    store.insert(document.id.clone(), document);
    for fusion in ["blend", "rrf"] {
        let options = SearchOptions {
            fusion: Some(fusion.into()),
            ..Default::default()
        };
        assert!(search_impl(&store, "ghost", &options, None, true)
            .unwrap()
            .is_empty());
        for query in ["ghost common", "ghost rare0"] {
            assert_eq!(
                serde_json::to_value(search_impl(&store, query, &options, None, true).unwrap())
                    .unwrap(),
                serde_json::to_value(search_impl(&store, query, &options, None, false).unwrap())
                    .unwrap()
            );
        }
    }
    // Counts agree: a zero-frequency posting is no match, through the multichunk bitmap
    // and through the one-chunk-per-document shortcut.
    assert_eq!(count_matches(&store, "ghost", Analyzer::Legacy).unwrap(), 0);
    assert_eq!(
        count_matches(&store, "ghost common", Analyzer::Legacy).unwrap(),
        1
    );
    let mut single = DocumentStore::default();
    // `document` names the moved local here; the helper is reached by path.
    let mut document = self::document(1, 1);
    document.chunks.truncate(1);
    document.chunks[0].terms.insert("ghost".into(), 0);
    single.insert(document.id.clone(), document);
    assert_eq!(
        count_matches(&single, "ghost", Analyzer::Legacy).unwrap(),
        0
    );
    assert_eq!(
        count_matches(&single, "ghost common", Analyzer::Legacy).unwrap(),
        1
    );
}

#[test]
fn embedding_normalization_checks_dimensions_and_extreme_values() {
    for dimensions in [1, 2, 3] {
        let normalized = normalize_embedding(&[3.0_f64, 4.0], dimensions);
        if dimensions == 2 {
            assert_eq!(normalized.unwrap(), [0.6, 0.8]);
        } else {
            assert!(normalized.is_err());
        }
    }
    assert_eq!(normalize_embedding(&[3.0_f32, 4.0], 2).unwrap(), [0.6, 0.8]);
    assert_eq!(
        normalize_embedding(&[f64::MAX, f64::MIN_POSITIVE], 2).unwrap(),
        [1.0, 0.0]
    );
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(normalize_embedding(&[value, 1.0], 2).is_err());
    }
    assert!(normalize_embedding(&[0.0_f64, -0.0], 2).is_err());
}

#[test]
fn embeddings_round_to_f32_and_read_legacy_f64_records() {
    let legacy = "[0.1, 0.2, 0.30000000000000004, -0.7071067811865476]";
    let read: Embedding = serde_json::from_str(legacy).unwrap();
    assert_eq!(
        read,
        Embedding::from_f32(vec![0.1, 0.2, 0.3, -std::f32::consts::FRAC_1_SQRT_2])
    );
    let written = serde_json::to_string(&read).unwrap();
    assert_eq!(written, "[0.1,0.2,0.3,-0.70710677]");
    let again: Embedding = serde_json::from_str(&written).unwrap();
    assert_eq!(again, read);
    assert_eq!(
        read.to_f64(),
        read.iter().map(|&v| f64::from(v)).collect::<Vec<_>>()
    );

    let stored = document(5, 1);
    let embedded = stored.chunks[0].embedding.as_ref().unwrap();
    let reference = normalize_embedding(&[1.0, 0.1, 0.0], 3).unwrap();
    assert_eq!(
        &embedded[..],
        &reference.iter().map(|&v| v as f32).collect::<Vec<_>>()[..]
    );
    let query = normalize_embedding(&[0.3, -0.2, 0.9], 3).unwrap();
    let exact: f64 = query.iter().zip(&reference).map(|(q, v)| q * v).sum();
    assert!((embedded.cosine(&query) - exact).abs() < 1e-6);
    let widened: f64 = query
        .iter()
        .zip(embedded.iter())
        .map(|(q, &v)| q * f64::from(v))
        .sum();
    assert_eq!(embedded.cosine(&query), widened.clamp(-1.0, 1.0));

    let json = serde_json::to_value(&stored).unwrap();
    assert!(json["chunks"][0]["embedding"].is_array());
    let parsed: StoredDocument = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.chunks[0].embedding, stored.chunks[0].embedding);
    let without: StoredChunk = {
        let mut chunk = stored.chunks[0].clone();
        chunk.embedding = None;
        chunk
    };
    assert_eq!(
        serde_json::to_value(&without).unwrap()["embedding"],
        Value::Null
    );
}

#[test]
fn replacements_preserve_frequency_filters_embeddings_and_retained_views() {
    for chunks in [1, 2, 3] {
        for change_tokenization in [false, true] {
            let mut store = DocumentStore::default();
            for i in 0..20 {
                let mut doc = document(i, 1);
                if i == 3 {
                    doc.chunks.truncate(2);
                }
                store.insert(doc.id.clone(), doc);
            }
            store.remove("d000");
            let options = SearchOptions {
                k: Some(100),
                candidate_limit: Some(100),
                embedding: Some(vec![1.0, 0.0, 0.0]),
                ..Default::default()
            };
            let prepared = prepare_search("running common ghost", &options, Some(3)).unwrap();
            let retained = capture_search(&store, vec![prepared]);
            let old = serde_json::to_value(execute_search(&retained, 0).unwrap()).unwrap();
            let mut replacement = document(3, 2);
            replacement.chunks.truncate(chunks);
            replacement.metadata = json!({"group":99, "flag":false}).into();
            replacement.source_uri = Some("replacement-source".into());
            for chunk in &mut replacement.chunks {
                chunk.terms.insert("common".into(), (chunks - 1) as u32);
                chunk.terms.insert("ghost".into(), 2);
                chunk.terms.remove("run");
                chunk.word_count += 1;
                chunk.embedding = None;
                if change_tokenization {
                    chunk.stemmed = !chunk.stemmed;
                }
            }
            store.insert(replacement.id.clone(), replacement);
            assert_scan_parity(&store);
            let captured = capture_search(
                &store,
                vec![prepare_search("running common ghost", &options, Some(3)).unwrap()],
            );
            assert_eq!(
                serde_json::to_value(execute_search(&captured, 0).unwrap()).unwrap(),
                serde_json::to_value(
                    search_impl(&store, "running common ghost", &options, Some(3), false).unwrap()
                )
                .unwrap()
            );
            assert_eq!(
                serde_json::to_value(execute_search(&retained, 0).unwrap()).unwrap(),
                old
            );
            assert_eq!(
                store.embedded_count(),
                store
                    .values()
                    .flat_map(|d| &d.chunks)
                    .filter(|c| c.embedding.is_some())
                    .count()
            );
            assert_eq!(
                store.word_count,
                store
                    .values()
                    .flat_map(|d| &d.chunks)
                    .map(|c| c.word_count)
                    .sum::<usize>()
            );
            assert_eq!(
                store.chunk_count,
                store.values().map(|d| d.chunks.len()).sum::<usize>()
            );
            for form in &store.postings {
                form.assert_valid();
            }
            let selected = search_impl(
                &store,
                "ghost",
                &SearchOptions {
                    source_uri: Some("replacement-source".into()),
                    filter: Some(json!({"group":99})),
                    k: Some(100),
                    candidate_limit: Some(100),
                    ..Default::default()
                },
                Some(3),
                true,
            )
            .unwrap();
            assert_eq!(selected.len(), chunks);
            assert!(selected.iter().all(|r| r.document_id == "d003"
                && r.version == "2"
                && r.semantic_score.is_none()));
            let mut embedded = document(3, 3);
            embedded.chunks.truncate(chunks);
            store.insert(embedded.id.clone(), embedded);
            assert_scan_parity(&store);
            assert!(store.embedded_documents.contains("d003"));
            assert_eq!(
                store.embedded_count(),
                store
                    .values()
                    .flat_map(|d| &d.chunks)
                    .filter(|c| c.embedding.is_some())
                    .count()
            );
            store.remove("d003");
            assert_scan_parity(&store);
            assert_eq!(
                serde_json::to_value(execute_search(&retained, 0).unwrap()).unwrap(),
                old
            );
        }
    }
}
