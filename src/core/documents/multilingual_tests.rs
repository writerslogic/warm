// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Store analyzers: the legacy analysis stays exactly what every store has always used, the
//! language analyzers conflate what their language inflects, and the analyzer a store was
//! created with is the one it keeps.

use super::{terms, terms_with, Analyzer, DocumentInput, EncodeMode, IngestOptions, SearchOptions};
use crate::core::WarmConfig;
use crate::WarmCore;
use proptest::prelude::*;

/// Spellings the English analyzer makes one term (the legacy one does not).
const PAIRS: &[(&str, &str, &str)] = &[
    ("nfc-nfd", "caf\u{E9}", "cafe\u{301}"),
    ("nfd-upper", "NAI\u{308}VE", "na\u{EF}ve"),
    ("turkish-dotted", "İstanbul", "istanbul"),
    ("turkish-dotted-nfd", "I\u{307}ZMIR", "izmir"),
    ("turkish-dotless", "IŞIK", "ışık"),
];

/// The analysis of every store written before analyzers existed (a090f971).
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

fn one(analyzer: Analyzer, word: &str, stem: bool) -> String {
    let terms: Vec<String> = terms_with(analyzer, word, stem).into_keys().collect();
    assert_eq!(terms.len(), 1, "{analyzer} {word:?} -> {terms:?}");
    terms.into_iter().next().unwrap()
}

#[test]
fn english_analyzer_folds_case_and_normalization() {
    for stem in [false, true] {
        for &(name, a, b) in PAIRS {
            assert_eq!(
                one(Analyzer::English, a, stem),
                one(Analyzer::English, b, stem),
                "{name}"
            );
        }
    }
    // The legacy analyzer keeps them apart, as it always has.
    assert_ne!(terms("caf\u{E9}", false), terms("cafe\u{301}", false));
}

#[test]
fn language_stemmers_conflate_inflections() {
    let pairs = [
        (Analyzer::Spanish, "gatos", "gato"),
        (Analyzer::Russian, "книги", "книга"),
        (Analyzer::German, "Häuser", "Haus"),
        (Analyzer::Finnish, "talossa", "talo"),
        (Analyzer::Arabic, "الكتاب", "كتاب"),
        (Analyzer::French, "chanteuses", "chanteuse"),
        // Persian: Arabic kaf and Persian keheh, and the plural suffix -ها.
        (Analyzer::Persian, "كتابها", "کتاب"),
        // Hindi: nukta, and the oblique plural -ों against the -े form.
        (Analyzer::Hindi, "लड़कों", "लडके"),
    ];
    for (analyzer, a, b) in pairs {
        assert_eq!(one(analyzer, a, true), one(analyzer, b, true), "{analyzer}");
    }
    // The English stemmer does not reduce the Spanish plural to the Spanish stem.
    assert_ne!(
        one(Analyzer::English, "gatos", true),
        one(Analyzer::Spanish, "gato", true)
    );
}

#[test]
fn persian_and_hindi_normalize_written_variants() {
    // ZWNJ ends a word, as Lucene's PersianCharFilter makes it; Farsi yeh meets Arabic yeh.
    let zwnj: Vec<String> = terms_with(Analyzer::Persian, "می\u{200C}روم", false)
        .into_keys()
        .collect();
    assert_eq!(zwnj, ["روم", "مي"]);
    assert_eq!(
        one(Analyzer::Persian, "یک", false),
        one(Analyzer::Persian, "يک", false)
    );
    // Dead n (na + virama) is anusvara; a conjunct stays one word.
    assert_eq!(
        one(Analyzer::Hindi, "हिन्दी", false),
        one(Analyzer::Hindi, "हिंदी", false)
    );
    assert_eq!(
        terms("हिन्दी", false).len(),
        2,
        "legacy splits at the virama"
    );
    // Devanagari digits are ASCII digits.
    assert_eq!(one(Analyzer::Hindi, "२०२६", false), "2026");
}

#[test]
fn bigram_analyzer_retrieves_thai_inside_unsegmented_text() {
    let document: Vec<String> = terms_with(Analyzer::Bigram, "ภาษาไทย ok", false)
        .into_keys()
        .collect();
    for term in terms_with(Analyzer::Bigram, "ไทย", false).into_keys() {
        assert!(document.contains(&term), "{term:?} not in {document:?}");
    }
    assert!(document.contains(&"ok".to_owned()));
    // A tone mark stays with its consonant rather than splitting the word.
    let marked: Vec<String> = terms_with(Analyzer::Bigram, "ไม่", false)
        .into_keys()
        .collect();
    assert_eq!(marked, ["ไม่"]);
}

/// Ingest's interned fast path gives every analyzer exactly the terms of the reference path.
#[test]
fn interned_terms_match_reference_for_every_analyzer() {
    let texts = [
        "Running runners RUN quickly; ΟΔΟΣ Οδός naïve cafe\u{301} İstanbul IŞIK 42",
        "می\u{200C}روم كتابها کتاب\u{0640}ها یک",
        "हिन्दी लड़कों के लिए २०२६ में",
        "ภาษาไทย ไม่ ok 日本語のテキスト",
        "gatos книги Häuser talossa الكتاب",
    ];
    for &(analyzer, _) in super::analysis::tests_names() {
        for stem in [false, true] {
            let mut memo = super::StemMemo::default();
            let _installed = memo.install();
            for text in texts {
                let (stored, words) = super::analyze_chunk(text, 1, stem, Some(false), analyzer);
                let got: Vec<(String, u32)> = (&stored)
                    .into_iter()
                    .map(|(k, &v)| (k.to_owned(), v))
                    .collect();
                let want: Vec<(String, u32)> =
                    terms_with(analyzer, text, stem).into_iter().collect();
                assert_eq!(got, want, "{analyzer} stem {stem} {text:?}");
                assert_eq!(
                    words,
                    analyzer.tokens(text, stem).len(),
                    "{analyzer} {text:?}"
                );
            }
        }
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

fn open(path: &std::path::Path, analyzer: Option<Analyzer>) -> anyhow::Result<WarmCore> {
    WarmCore::new(
        1024,
        Some(path.display().to_string()),
        Some(WarmConfig {
            analyzer,
            ..Default::default()
        }),
    )
}

fn found(core: &WarmCore, query: &str) -> Vec<String> {
    let search = SearchOptions {
        k: Some(10),
        candidate_limit: Some(10),
        lexical_weight: Some(1.0),
        semantic_weight: Some(0.0),
        ..Default::default()
    };
    core.search_documents(query, &search)
        .unwrap()
        .into_iter()
        .map(|r| r.document_id)
        .collect()
}

fn lexical() -> IngestOptions {
    IngestOptions {
        encode: EncodeMode::Lexical,
        positions: true,
        ..Default::default()
    }
}

/// A store created without an analyzer records none and analyzes exactly as before: a
/// decomposed query still misses a precomposed document, across reopen and compaction.
#[test]
fn default_store_keeps_the_legacy_analysis() {
    let dir = tempfile::tempdir().unwrap();
    {
        let core = open(dir.path(), None).unwrap();
        assert_eq!(core.analyzer(), Analyzer::Legacy);
        core.memorize_documents_with(vec![document("cafe", "caf\u{E9} running")], &lexical())
            .unwrap();
        assert_eq!(found(&core, "runs"), ["cafe"]);
        assert!(found(&core, "cafe\u{301}").is_empty());
    }
    let schema = std::fs::read_to_string(dir.path().join("store.json")).unwrap();
    assert!(!schema.contains("lexical_analyzer"), "{schema}");
    let core = open(dir.path(), None).unwrap();
    core.compact().unwrap();
    assert_eq!(core.analyzer(), Analyzer::Legacy);
    assert_eq!(found(&core, "caf\u{E9}"), ["cafe"]);
    assert!(found(&core, "cafe\u{301}").is_empty());
    drop(core);
    assert!(open(dir.path(), Some(Analyzer::English)).is_err());
}

/// The analyzer a store is created with is recorded, governs ingest and every query path,
/// and survives reopen and compaction; reopening with another one fails.
#[test]
fn store_analyzer_persists_across_reopen_and_compaction() {
    let dir = tempfile::tempdir().unwrap();
    {
        let core = open(dir.path(), Some(Analyzer::Spanish)).unwrap();
        core.memorize_documents_with(
            vec![
                document("gatos", "los gatos negros"),
                document("perro", "un perro"),
            ],
            &lexical(),
        )
        .unwrap();
        assert_eq!(found(&core, "gato"), ["gatos"]);
    }
    let core = open(dir.path(), None).unwrap();
    assert_eq!(core.analyzer(), Analyzer::Spanish);
    assert_eq!(found(&core, "gato"), ["gatos"]);
    core.compact().unwrap();
    drop(core);
    let core = open(dir.path(), Some(Analyzer::Spanish)).unwrap();
    assert_eq!(found(&core, "gato negro"), ["gatos"]);
    assert_eq!(core.count_documents("gato").unwrap(), 1);
    let phrase = super::LexicalQuery::parse("\"gato negro\"").unwrap();
    let hits = core
        .search_lexical(&phrase, &super::LexicalOptions::default())
        .unwrap();
    assert_eq!(hits.hits.len(), 1);
    drop(core);
    let error = open(dir.path(), Some(Analyzer::English)).err().unwrap();
    assert!(error.to_string().contains("spanish"), "{error}");
}

#[test]
fn analyzer_names_round_trip() {
    for &(analyzer, name) in super::analysis::tests_names() {
        assert_eq!(analyzer.name(), name);
        assert_eq!(name.parse::<Analyzer>().unwrap(), analyzer);
    }
    assert!("klingon".parse::<Analyzer>().is_err());
}

proptest! {
    /// The legacy analyzer is a090f971's analysis on any text.
    #[test]
    fn legacy_analysis_is_unchanged(text in "\\PC{0,120}", stem: bool) {
        prop_assert_eq!(terms(&text, stem), legacy_terms(&text, stem));
    }

    /// The English analyzer is the legacy one on ASCII text.
    #[test]
    fn english_analyzer_is_legacy_on_ascii(text in "[\\x00-\\x7f]{0,200}", stem: bool) {
        prop_assert_eq!(terms_with(Analyzer::English, &text, stem), legacy_terms(&text, stem));
    }
}
