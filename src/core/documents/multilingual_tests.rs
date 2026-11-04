// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Case and normalization folding outside ASCII: spellings that differ only in case or
//! Unicode normalization form meet at one term, through ingest and search.
//!
//! Combining marks and ZWNJ still end a word. Keeping them inside words (one term per Hindi,
//! Persian or Thai word) was built and measured: without per-language stemming it lowered
//! MIRACL dev nDCG@10 on hi, fa and th (`benchmarks/results/multilingual_bm25_miracl_dev.json`,
//! `warm-joined`), so the old split stays.

use super::{terms, DocumentInput, EncodeMode, IngestOptions, SearchOptions};
use crate::WarmCore;
use proptest::prelude::*;

/// Spellings that must be one term each and the same term as each other.
const PAIRS: &[(&str, &str, &str)] = &[
    ("nfc-nfd", "caf\u{E9}", "cafe\u{301}"),
    ("nfd-upper", "NAI\u{308}VE", "na\u{EF}ve"),
    ("turkish-dotted", "İstanbul", "istanbul"),
    ("turkish-dotted-nfd", "I\u{307}ZMIR", "izmir"),
    ("turkish-dotless", "IŞIK", "ışık"),
];

/// What the analyzer did before marks, ZWNJ and normalization were handled; kept to pin
/// that ASCII text analyzes exactly as it always has.
fn legacy_terms(text: &str, stem: bool) -> std::collections::BTreeMap<String, u32> {
    let stemmer = rust_stemmers::Stemmer::create(rust_stemmers::Algorithm::English);
    let mut out = std::collections::BTreeMap::new();
    for word in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
    {
        let lower = word.to_lowercase();
        let key = if stem {
            stemmer.stem(&lower).into_owned()
        } else {
            lower
        };
        *out.entry(key).or_insert(0) += 1;
    }
    out
}

#[test]
fn differently_cased_and_normalized_spellings_are_one_term() {
    for stem in [false, true] {
        for &(name, a, b) in PAIRS {
            let (ta, tb) = (terms(a, stem), terms(b, stem));
            assert_eq!(ta.len(), 1, "{name}: {a:?} -> {ta:?}");
            assert_eq!(ta, tb, "{name}: {a:?} vs {b:?}");
        }
    }
    // Decomposed text normalizes to the precomposed term, whichever form is indexed.
    assert_eq!(
        terms("cafe\u{301}", false).into_keys().collect::<Vec<_>>(),
        ["caf\u{E9}"]
    );
}

#[test]
fn differently_cased_and_normalized_spellings_retrieve_one_document() {
    let dir = tempfile::tempdir().unwrap();
    let core = WarmCore::new(1024, Some(dir.path().display().to_string()), None).unwrap();
    let mut documents = Vec::new();
    for &(name, a, _) in PAIRS {
        documents.push(document(&format!("{name}-word"), &format!("{a} filler")));
    }
    let options = IngestOptions {
        encode: EncodeMode::Lexical,
        ..Default::default()
    };
    core.memorize_documents_with(documents, &options).unwrap();
    let search = SearchOptions {
        k: Some(10),
        candidate_limit: Some(10),
        lexical_weight: Some(1.0),
        semantic_weight: Some(0.0),
        ..Default::default()
    };
    for &(name, _, b) in PAIRS {
        let found: Vec<String> = core
            .search_documents(b, &search)
            .unwrap()
            .into_iter()
            .map(|r| r.document_id)
            .collect();
        assert_eq!(found, [format!("{name}-word")], "{name}: query {b:?}");
    }
}

fn document(id: &str, text: &str) -> DocumentInput {
    DocumentInput {
        id: id.into(),
        text: text.into(),
        source_uri: None,
        version: None,
        metadata: None,
        chunk_words: None,
        overlap_words: None,
        store_text: Some(true),
        embeddings: None,
        title_weight: None,
    }
}

#[test]
fn ascii_text_analyzes_exactly_as_before() {
    let sample = "The quick brown fox's re-entry: 3.14, x86_64 & co-op LLMs, \
                  e-mail@example.com (RUNNING runners ran) 1990s; naive ASCII only.";
    for stem in [false, true] {
        assert_eq!(terms(sample, stem), legacy_terms(sample, stem));
    }
}

proptest! {
    #[test]
    fn any_ascii_text_analyzes_exactly_as_before(text in "[\\x00-\\x7f]{0,200}", stem: bool) {
        prop_assert_eq!(terms(&text, stem), legacy_terms(&text, stem));
    }
}
