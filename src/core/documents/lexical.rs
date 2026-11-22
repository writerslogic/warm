// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::BTreeSet;

use super::packed_terms::WordId;
use super::{Bm25, DocumentStore, SearchOptions, StoredChunk, MAX_DOCUMENT_BYTES};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LexicalExecution {
    #[default]
    Auto,
    /// Unpruned posting-cursor execution.
    DocumentAtATime,
    /// Dynamic pruning for term disjunctions (block-max MaxScore, or a strongest-blocks-first
    /// scan of a single list); other queries use cursors.
    BlockMaxWand,
    /// Full scan and full sort, useful for verifying another execution strategy.
    Exhaustive,
}

#[derive(Clone, Debug)]
pub struct LexicalOptions {
    pub k: usize,
    pub count: bool,
    pub bm25_k1: f64,
    pub bm25_b: f64,
    pub execution: LexicalExecution,
}

impl Default for LexicalOptions {
    fn default() -> Self {
        Self {
            k: 10,
            count: false,
            bm25_k1: 1.2,
            bm25_b: 0.75,
            execution: LexicalExecution::Auto,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LexicalHit {
    pub document_id: String,
    pub chunk_index: usize,
    pub score: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LexicalResults {
    pub hits: Vec<LexicalHit>,
    pub count: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Occur {
    Must,
    Should,
    MustNot,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Clause {
    occur: Occur,
    words: [Vec<String>; 2],
    phrase: bool,
}

/// A flat Boolean query: bare optional clauses, +required clauses, -excluded clauses,
/// and quoted adjacent phrases. Purely negative queries match no documents.
#[derive(Clone, Debug)]
pub struct LexicalQuery {
    clauses: Vec<Clause>,
    text: String,
    analyzer: super::Analyzer,
}

impl LexicalQuery {
    /// `text` parsed and analyzed as a [`super::Analyzer::Legacy`] store analyzes it; a store
    /// with another analyzer analyzes it again (`Self::for_analyzer`).
    pub fn parse(text: &str) -> Result<Self> {
        Self::parse_with(text, super::Analyzer::Legacy)
    }

    /// The query as a store with `analyzer` matches it.
    pub(crate) fn for_analyzer(&self, analyzer: super::Analyzer) -> Result<Cow<'_, Self>> {
        if analyzer == self.analyzer {
            Ok(Cow::Borrowed(self))
        } else {
            Self::parse_with(&self.text, analyzer).map(Cow::Owned)
        }
    }

    fn parse_with(text: &str, analyzer: super::Analyzer) -> Result<Self> {
        ensure!(text.len() <= MAX_DOCUMENT_BYTES, "query exceeds 8 MiB");
        let mut rest = text.trim();
        let mut clauses = Vec::new();
        while !rest.is_empty() {
            ensure!(clauses.len() < 1024, "lexical query exceeds 1024 clauses");
            let occur = match rest.as_bytes()[0] {
                b'+' => {
                    rest = &rest[1..];
                    Occur::Must
                }
                b'-' => {
                    rest = &rest[1..];
                    Occur::MustNot
                }
                _ => Occur::Should,
            };
            let phrase = rest.starts_with('"');
            let value;
            if phrase {
                rest = &rest[1..];
                let end = rest
                    .find('"')
                    .ok_or_else(|| anyhow::anyhow!("unterminated lexical phrase"))?;
                value = &rest[..end];
                rest = &rest[end + 1..];
                ensure!(
                    rest.is_empty() || rest.starts_with(char::is_whitespace),
                    "unsupported lexical phrase suffix (only zero slop is supported)"
                );
                ensure!(
                    value
                        .chars()
                        .all(|c| c.is_alphanumeric() || c.is_whitespace()),
                    "unsupported character in lexical phrase"
                );
            } else {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                value = &rest[..end];
                rest = &rest[end..];
                ensure!(
                    !value.is_empty() && value.chars().all(char::is_alphanumeric),
                    "unsupported lexical query syntax"
                );
            }
            let words = [analyzer.tokens(value, false), analyzer.tokens(value, true)];
            ensure!(!words[0].is_empty(), "lexical clause requires a token");
            ensure!(words[0].len() <= 1024, "lexical phrase exceeds 1024 tokens");
            clauses.push(Clause {
                occur,
                phrase: phrase && words[0].len() > 1,
                words,
            });
            rest = rest.trim_start();
        }
        clauses.sort_unstable();
        clauses.dedup();
        Ok(Self {
            clauses,
            text: text.to_owned(),
            analyzer,
        })
    }

    fn has_positive(&self) -> bool {
        self.clauses.iter().any(|c| c.occur != Occur::MustNot)
    }

    /// The distinct terms per form of a query that is a bare disjunction of single words, the
    /// terms [`super::query_words_both`] reads from the same text; `None` for a query with a
    /// required, excluded or phrase clause.
    fn disjunction_terms(&self) -> Option<[BTreeSet<String>; 2]> {
        let mut terms: [BTreeSet<String>; 2] = Default::default();
        for clause in &self.clauses {
            if clause.occur != Occur::Should || clause.phrase {
                return None;
            }
            for (form, set) in terms.iter_mut().enumerate() {
                let [word] = clause.words[form].as_slice() else {
                    return None;
                };
                set.insert(word.clone());
            }
        }
        (!self.clauses.is_empty()).then_some(terms)
    }
}

/// The search `WarmCore::search_documents` runs for the text of a bare disjunction, so both ranks one way: the same terms, per-slot BM25 sums and tie order.
/// `None` when `query` is not such a disjunction or `options` ask for a count, a chosen
/// execution strategy or a `k` outside 1 to 1000; those run through [`execute`].
pub(crate) fn plain_search(
    query: &LexicalQuery,
    options: &LexicalOptions,
) -> Result<Option<super::PreparedSearch>> {
    if options.count
        || options.execution != LexicalExecution::Auto
        || !(1..=1000).contains(&options.k)
    {
        return Ok(None);
    }
    let Some(terms) = query.disjunction_terms() else {
        return Ok(None);
    };
    prepare(options)?;
    let defaults = LexicalOptions::default();
    let custom = options.bm25_k1 != defaults.bm25_k1 || options.bm25_b != defaults.bm25_b;
    super::search::prepare_terms(
        terms,
        &SearchOptions {
            k: Some(options.k as u32),
            candidate_limit: Some(options.k as u32),
            bm25_k1: custom.then_some(options.bm25_k1),
            bm25_b: custom.then_some(options.bm25_b),
            ..Default::default()
        },
    )
    .map(Some)
}

/// The clause's occurrences in `chunk`: a word's frequency, or a phrase's when `exact`,
/// otherwise 1 at its first occurrence. `ids` holds one cache per phrase word and form.
#[cfg(test)]
fn frequency(clause: &Clause, chunk: &StoredChunk, exact: bool, ids: &mut [Vec<WordId>; 2]) -> u32 {
    frequency_with_positions(clause, chunk, exact, ids, chunk.terms.positions.as_deref())
}

fn frequency_with_positions(
    clause: &Clause,
    chunk: &StoredChunk,
    exact: bool,
    ids: &mut [Vec<WordId>; 2],
    positions: Option<&super::positions::Positions>,
) -> u32 {
    let form = usize::from(chunk.stemmed);
    let words = &clause.words[form];
    if !clause.phrase {
        return chunk.terms.get(&words[0]).copied().unwrap_or(0);
    }
    let mut indices = Vec::with_capacity(words.len());
    let (mut lead, mut least) = (0, u32::MAX);
    for (at, (word, id)) in words.iter().zip(&mut ids[form]).enumerate() {
        // A repeated word of a short phrase reuses its earlier index.
        let earlier = (words.len() <= 16)
            .then(|| words[..at].iter().position(|w| w == word))
            .flatten();
        let found = match earlier {
            Some(earlier) => Some(indices[earlier]),
            None => chunk.terms.index_cached(word, id),
        };
        let Some(index) = found else {
            return 0;
        };
        let frequency = chunk.terms.frequency_at(index);
        if frequency == 0 {
            return 0;
        }
        if frequency < least {
            lead = indices.len();
            least = frequency;
        }
        indices.push(index);
    }
    let positions = positions.expect("phrase capability checked");
    let mut cursors: Vec<_> = indices
        .iter()
        .map(|&i| positions.iter(i).peekable())
        .collect();
    let mut count = 0;
    for position in positions.iter(indices[lead]) {
        let Some(start) = position.checked_sub(lead as u32) else {
            continue;
        };
        let matched = cursors.iter_mut().enumerate().all(|(offset, iter)| {
            let Some(target) = start.checked_add(offset as u32) else {
                return false;
            };
            while iter.peek().is_some_and(|&p| p < target) {
                iter.next();
            }
            iter.peek() == Some(&target)
        });
        count += u32::from(matched);
        if matched && !exact {
            break;
        }
    }
    count
}

fn positions_for_chunk<'a>(
    store: &DocumentStore,
    chunk: &'a StoredChunk,
) -> Result<Cow<'a, super::positions::Positions>> {
    match chunk.terms.positions.as_deref() {
        Some(positions) => Ok(Cow::Borrowed(positions)),
        None => {
            anyhow::ensure!(
                !chunk.analyzed,
                "unsupported index: a phrase over a chunk without stored positions needs the legacy analyzer"
            );
            let text = store.try_chunk_text(chunk)?.context("unsupported index: a candidate phrase needs stored positions or retained chunk text")?;
            Ok(Cow::Owned(super::positions::Positions::from_text(
                &text,
                chunk.stemmed,
                &chunk.terms,
                chunk.terms.is_whole_document(),
            )))
        }
    }
}

/// One dictionary-id cache per word and form of each clause: a dictionary may hold chunks
/// of both forms, whose words differ.
fn word_ids(query: &LexicalQuery) -> Vec<[Vec<WordId>; 2]> {
    query
        .clauses
        .iter()
        .map(|clause| {
            std::array::from_fn(|form| {
                clause.words[form]
                    .iter()
                    .map(|_| WordId::default())
                    .collect()
            })
        })
        .collect()
}

fn prepare(options: &LexicalOptions) -> Result<Bm25> {
    ensure!(options.k <= 1_000_000, "lexical k exceeds 1000000");
    let bm25 = Bm25::from_options(&SearchOptions {
        bm25_k1: Some(options.bm25_k1),
        bm25_b: Some(options.bm25_b),
        ..Default::default()
    })?;
    Ok(bm25)
}

fn clause_idfs(store: &DocumentStore, query: &LexicalQuery) -> Vec<[f64; 2]> {
    let n = store.chunk_count as f64;
    query
        .clauses
        .iter()
        .map(|clause| {
            std::array::from_fn(|form| {
                clause.words[form]
                    .iter()
                    .map(|word| {
                        let df = store.postings[form].get(word).map_or(0, |l| l.len()) as f64;
                        Bm25::idf(n, df)
                    })
                    .sum()
            })
        })
        .collect()
}

fn matches(query: &LexicalQuery, frequencies: &[u32]) -> bool {
    let mut required = false;
    let mut optional = false;
    for (clause, &frequency) in query.clauses.iter().zip(frequencies) {
        match clause.occur {
            Occur::Must => {
                required = true;
                if frequency == 0 {
                    return false;
                }
            }
            Occur::MustNot if frequency != 0 => return false,
            Occur::Should => optional |= frequency != 0,
            _ => {}
        }
    }
    required || optional
}

fn score(
    query: &LexicalQuery,
    frequencies: &[u32],
    idfs: &[[f64; 2]],
    form: usize,
    norm: f64,
    bm25: Bm25,
) -> Result<f64> {
    let mut score = 0.0;
    for ((clause, &frequency), idf) in query.clauses.iter().zip(frequencies).zip(idfs) {
        if clause.occur != Occur::MustNot && frequency > 0 {
            score += bm25.weight(idf[form], f64::from(frequency), norm);
        }
    }
    ensure!(
        score.is_finite(),
        "lexical score is nonfinite; reduce BM25 parameters"
    );
    Ok(score)
}

fn order_hits(a: &LexicalHit, b: &LexicalHit) -> std::cmp::Ordering {
    b.score
        .total_cmp(&a.score)
        .then_with(|| a.document_id.cmp(&b.document_id))
        .then_with(|| a.chunk_index.cmp(&b.chunk_index))
}

/// The result of `query` ranked and counted from the checkpoint under `store`, when `store` is
/// that checkpoint alone and the query is one [`served::execute`] answers; `None` when the
/// live store must answer it.
pub(crate) fn served(
    store: &DocumentStore,
    query: &LexicalQuery,
    options: &LexicalOptions,
) -> Option<Result<LexicalResults>> {
    let base = store.base.as_ref()?;
    if !base.intact() || !store.documents.is_empty() {
        return None;
    }
    let bm25 = match prepare(options) {
        Ok(bm25) => bm25,
        Err(error) => return Some(Err(error)),
    };
    // A bare disjunction the block-max executor does not take is ranked as `search_documents`
    // ranks the same words.
    let mut prepared_search = None;
    // `plain_search` prepares nothing for `k == 0`.
    if options.k > 0 && query.disjunction_terms().is_some() {
        match plain_search(
            query,
            &LexicalOptions {
                count: false,
                ..options.clone()
            },
        ) {
            Ok(Some(prepared)) => {
                if !super::overlay_serves(store, std::slice::from_ref(&prepared)) {
                    return None;
                }
                prepared_search = Some(prepared);
            }
            Ok(None) => {}
            Err(error) => return Some(Err(error)),
        }
    }
    let ranker = prepared_search
        .as_ref()
        .map(|prepared| move || super::rank_lexical_overlaid(store, prepared).hits);
    served::execute(
        &base.image,
        query,
        options,
        bm25,
        ranker.as_ref().map(|f| f as &dyn Fn() -> Vec<LexicalHit>),
    )
    .transpose()
}

pub(crate) fn execute(
    store: &DocumentStore,
    query: &LexicalQuery,
    options: &LexicalOptions,
) -> Result<LexicalResults> {
    let bm25 = prepare(options)?;
    if options.execution != LexicalExecution::Exhaustive
        && options.k == 0
        && options.count
        && query
            .clauses
            .iter()
            .all(|clause| clause.occur == Occur::Should && !clause.phrase)
    {
        return Ok(LexicalResults {
            hits: Vec::new(),
            count: Some(execution::count_union(store, query)),
        });
    }
    match options.execution {
        LexicalExecution::Exhaustive => exhaustive(store, query, options, bm25),
        LexicalExecution::Auto | LexicalExecution::BlockMaxWand
            if options.k > 0
                && (options.execution == LexicalExecution::BlockMaxWand || options.k <= 1000) =>
        {
            match wand::execute(store, query, options, bm25)? {
                Some(result) => Ok(result),
                None => document_at_a_time(store, query, options, bm25),
            }
        }
        _ => document_at_a_time(store, query, options, bm25),
    }
}

fn exhaustive(
    store: &DocumentStore,
    query: &LexicalQuery,
    options: &LexicalOptions,
    bm25: Bm25,
) -> Result<LexicalResults> {
    let mut hits = Vec::new();
    let mut matched = BTreeSet::new();
    if !query.has_positive() {
        return Ok(LexicalResults {
            hits,
            count: options.count.then_some(0),
        });
    }
    let idfs = if options.k > 0 {
        clause_idfs(store, query)
    } else {
        Vec::new()
    };
    let average = (store.word_count as f64 / (store.chunk_count as f64).max(1.0)).max(1.0);
    let mut ids = word_ids(query);
    for (id, document) in &store.documents {
        for (chunk_index, chunk) in document.chunks.iter().enumerate() {
            let possible = query.clauses.iter().any(|clause| {
                clause.phrase
                    && clause.words[usize::from(chunk.stemmed)].iter().all(|word| {
                        chunk
                            .terms
                            .get(word)
                            .is_some_and(|&frequency| frequency > 0)
                    })
            });
            let positions = possible
                .then(|| positions_for_chunk(store, chunk))
                .transpose()?;
            let frequencies: Vec<_> = query
                .clauses
                .iter()
                .zip(&mut ids)
                .map(|(clause, ids)| {
                    frequency_with_positions(
                        clause,
                        chunk,
                        options.k > 0,
                        ids,
                        positions.as_deref(),
                    )
                })
                .collect();
            if !matches(query, &frequencies) {
                continue;
            }
            if options.count {
                matched.insert(id);
            }
            if options.k > 0 {
                hits.push(LexicalHit {
                    document_id: id.clone(),
                    chunk_index,
                    score: score(
                        query,
                        &frequencies,
                        &idfs,
                        usize::from(chunk.stemmed),
                        bm25.norm(chunk.word_count as f64, average),
                        bm25,
                    )?,
                });
            }
        }
    }
    hits.sort_unstable_by(order_hits);
    hits.truncate(options.k);
    Ok(LexicalResults {
        hits,
        count: options.count.then_some(matched.len()),
    })
}

pub(super) mod execution;
pub(in crate::core::documents) mod served;
pub(in crate::core::documents) mod wand;
use execution::document_at_a_time;

#[cfg(test)]
mod tests;
