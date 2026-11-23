// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! The experience core behind the memory API.
//!
//! `remember` routes by shape: an object with an `episode` string, a `step` number and a `tree`
//! object is an observation and goes to the experience core; everything else is a fact and is
//! handled as before. `recall` merges the best regions with the document results by reciprocal
//! rank fusion. `explain` reports a region's provenance. `forget` removes an episode.
//!
//! One ranking contract. Regions are ranked by the lexical index of the experience core and
//! facts by the document index, so each list has its own corpus statistics and the scores of the
//! two are not on one scale. The merge therefore uses ranks only: a result at 1-based position
//! `r` in its list contributes `1 / (60 + r)`, the results are ordered by that value, ties go to
//! the document and then to the better rank, and `score` carries the fused value. A recall that
//! finds no region, or that sets `subject`, `entities`, `filter`, `knownAt` or historical mode
//! (which regions do not have), returns the document results exactly as before.

use anyhow::Result;
use serde_json::{Map, Value};

use super::recall::{matched_terms, query_terms};
use super::types::*;
use super::{HistoryMode, Memory, MemoryError};
use crate::core::experience::{
    check_observation_value, is_observation_object, is_observation_shape, rank_fusion_score,
    EffectHit, ExperienceError, Ingested, ObservationInput, OwnedObservation, RegionHit,
};

const REGION_PREFIX: &str = "experience:r";
const EPISODE_PREFIX: &str = "experience:episode:";
/// Regions retrieved to merge for explain's rank.
const EXPLAIN_REGIONS: usize = 100;

pub(super) fn region_id(id: &str) -> Option<u32> {
    id.strip_prefix(REGION_PREFIX)?.parse().ok()
}

pub(super) fn effect_id(id: &str) -> Option<&str> {
    id.strip_prefix("experience:")
        .filter(|r| r.starts_with('x') && r.contains(':'))
}

fn experience_error(error: ExperienceError) -> anyhow::Error {
    match error {
        ExperienceError::Conflict { .. } | ExperienceError::Unverifiable { .. } => {
            MemoryError::Conflict(error.to_string()).into()
        }
        ExperienceError::Limit { .. }
        | ExperienceError::Invalid(_)
        | ExperienceError::OutOfOrder { .. } => MemoryError::Invalid(error.to_string()).into(),
        other => anyhow::anyhow!(other),
    }
}

/// Whether a `remember` request carries an observation, alone or in `facts`.
pub(super) fn carries_observation(map: &Map<String, Value>) -> bool {
    match map.get("facts") {
        Some(Value::Array(facts)) => facts.iter().any(is_observation_shape),
        Some(_) => false,
        None => is_observation_object(map),
    }
}

impl Memory {
    /// Parses and bounds an observation before any tree is built from it.
    pub(super) fn parse_observation(&self, value: Value) -> Result<ObservationInput> {
        let limits = self.core.experience_limits();
        check_observation_value(&value, &limits).map_err(experience_error)?;
        ObservationInput::from_value(value)
            .map_err(|error| MemoryError::Invalid(format!("invalid observation: {error}")).into())
    }

    /// Stores observation steps in the experience core, in order. Each step is durable when the
    /// call returns. On a failing step the steps before it stay stored.
    pub fn remember_observations(
        &self,
        observations: Vec<OwnedObservation>,
    ) -> Result<Vec<Remembered>> {
        self.remember_observations_within(observations.into_iter().map(Into::into).collect(), None)
    }

    /// With a document `limit`, a step that would take the documents plus the experience units
    /// past it is refused with [`DocumentQuotaExceeded`](super::DocumentQuotaExceeded).
    pub(super) fn remember_observations_within(
        &self,
        observations: Vec<ObservationInput>,
        limit: Option<u64>,
    ) -> Result<Vec<Remembered>> {
        check!(
            !observations.is_empty(),
            "remember requires 1.. observations"
        );
        let now = self.clock.now_millis();
        let ids: Vec<String> = observations
            .iter()
            .map(|o| {
                format!(
                    "experience:{}:{}",
                    o.observation.episode, o.observation.step
                )
            })
            .collect();
        let outcomes = self
            .core
            .with_experience_mut(|store| {
                let budget = limit.map(|l| l.saturating_sub(self.core.document_count() as u64));
                store.ingest_inputs_within(observations, budget)
            })
            .map_err(experience_error)?
            .map_err(|(position, error)| {
                if let (ExperienceError::Quota(_), Some(limit)) = (&error, limit) {
                    return anyhow::Error::new(super::DocumentQuotaExceeded { limit });
                }
                let error = experience_error(error);
                match error.downcast_ref::<MemoryError>() {
                    Some(MemoryError::Invalid(m)) => anyhow::Error::new(MemoryError::Invalid(
                        format!("observation {position}: {m}"),
                    )),
                    Some(MemoryError::Conflict(m)) => anyhow::Error::new(MemoryError::Conflict(
                        format!("observation {position}: {m}"),
                    )),
                    _ => error,
                }
            })?;
        Ok(ids
            .into_iter()
            .zip(outcomes)
            .map(|(id, outcome)| Remembered {
                id,
                created: outcome == Ingested::Applied,
                supersedes: Vec::new(),
                superseded_by: Vec::new(),
                ingest_time: now,
            })
            .collect())
    }

    /// `remember` for a JSON request that carries observations. Observations are stored first,
    /// in order; facts follow in one transaction. The result lists every entry in input order.
    pub(super) fn remember_routed(
        &self,
        map: Map<String, Value>,
        limit: Option<u64>,
        parse_fact: impl Fn(Value) -> Result<RememberInput>,
    ) -> Result<Vec<Remembered>> {
        let items: Vec<Value> = match map.get("facts") {
            Some(Value::Array(_)) => {
                let mut map = map;
                match map.remove("facts") {
                    Some(Value::Array(items)) if map.is_empty() => items,
                    _ => invalid!("send either a fact or {{\"facts\": [..]}}, not both"),
                }
            }
            _ => vec![Value::Object(map)],
        };
        check!(
            !items.is_empty() && items.len() <= 4096,
            "remember requires 1..=4096 facts"
        );
        let mut facts = Vec::new();
        let mut observations = Vec::new();
        let mut slots = Vec::with_capacity(items.len());
        for item in items {
            if is_observation_shape(&item) {
                slots.push((true, observations.len()));
                observations.push(self.parse_observation(item)?);
            } else {
                slots.push((false, facts.len()));
                facts.push(parse_fact(item)?);
            }
        }
        // Observations first: a retried step is a no-op, so a request that fails after them can
        // be sent again. Facts are one transaction and go last; if they fail once observations
        // are stored the failure is reported as a committed mutation, never as a plain refusal.
        let observation_results = if observations.is_empty() {
            Vec::new()
        } else {
            self.remember_observations_within(observations, limit)?
        };
        let fact_results = if facts.is_empty() {
            Vec::new()
        } else {
            let stored = observation_results.iter().any(|r| r.created);
            let result = self.remember_many_with_limit(facts, limit);
            if stored {
                crate::core::error::after_commit("fact write", result)?
            } else {
                result?
            }
        };
        Ok(slots
            .into_iter()
            .map(|(observed, i)| {
                if observed {
                    observation_results[i].clone()
                } else {
                    fact_results[i].clone()
                }
            })
            .collect())
    }

    pub(super) fn forget_episode(
        &self,
        episode: &str,
        options: &ForgetOptions,
    ) -> Result<ForgetReport> {
        check!(
            !episode.is_empty() && !episode.contains('\0'),
            "episode must be a nonempty id"
        );
        let present = self
            .core
            .with_experience(|store| store.has_episode(episode))
            .unwrap_or(false);
        let forgotten = if present {
            vec![format!("{EPISODE_PREFIX}{episode}")]
        } else {
            Vec::new()
        };
        if present && !options.dry_run {
            self.core
                .with_experience_mut(|store| store.purge_episode(episode))
                .map_err(experience_error)?
                .map_err(experience_error)?;
        }
        Ok(ForgetReport {
            forgotten,
            dry_run: options.dry_run,
            purged: present && !options.dry_run,
        })
    }

    /// Merges the best regions into document results; see the module documentation.
    pub(super) fn merge_regions(
        &self,
        query: &str,
        options: &RecallOptions,
        mut documents: Vec<Recalled>,
    ) -> Result<Vec<Recalled>> {
        let history = options.history.unwrap_or(self.config.default_history);
        if options.subject.is_some()
            || options.entities.is_some()
            || options.filter.is_some()
            || options.known_at.is_some()
            || history == HistoryMode::Historical
            || !self
                .core
                .with_experience(|store| store.is_searchable())
                .unwrap_or(false)
        {
            return Ok(documents);
        }
        let k = options.k.unwrap_or(self.config.default_k) as usize;
        let (hits, effect_hits) = self
            .core
            .with_experience(|store| {
                Ok::<_, ExperienceError>((
                    store.search_regions(query, k)?,
                    store.search_effects(query, k)?,
                ))
            })
            .transpose()
            .map_err(experience_error)?
            .unwrap_or_default();
        if hits.is_empty() && effect_hits.is_empty() {
            return Ok(documents);
        }
        let terms = options
            .explain
            .then(|| query_terms(self.core.analyzer(), query));
        let fused = rank_fusion_score;
        for (i, d) in documents.iter_mut().enumerate() {
            d.score = fused(i + 1);
        }
        let mut merged = documents;
        for (i, hit) in hits.into_iter().enumerate() {
            let score = fused(i + 1);
            let explanation = terms.as_ref().map(|t| {
                region_explanation(&hit, Some(i + 1), Some(t), options, self.clock.now_millis())
            });
            merged.push(region_result(hit, score, explanation));
        }
        for (i, hit) in effect_hits.into_iter().enumerate() {
            let score = fused(i + 1);
            let explanation = terms.as_ref().map(|t| {
                effect_explanation(&hit, Some(i + 1), Some(t), options, self.clock.now_millis())
            });
            merged.push(effect_result(hit, score, explanation));
        }
        // Stable: on equal fused scores a document precedes a region and a better rank a worse.
        merged.sort_by(|a, b| b.score.total_cmp(&a.score));
        merged.truncate(k);
        for (i, r) in merged.iter_mut().enumerate() {
            if let Some(e) = r.explanation.as_mut() {
                e.rank = Some(i + 1);
                e.final_score = Some(r.score);
            }
        }
        Ok(merged)
    }

    pub(super) fn explain_effect(
        &self,
        query: &str,
        id: &str,
        options: &RecallOptions,
    ) -> Result<Explanation> {
        let found = self
            .core
            .with_experience(|store| {
                (
                    store.effect_hit(id),
                    store.search_effects(query, EXPLAIN_REGIONS),
                )
            })
            .and_then(|(hit, ranked)| hit.map(|h| (h, ranked)));
        let Some((mut hit, ranked)) = found else {
            return Err(MemoryError::NotFound(format!("unknown effect experience:{id}")).into());
        };
        let ranked = ranked.map_err(experience_error)?;
        let position = ranked.iter().position(|h| h.id == id);
        if let Some(p) = position {
            hit.score = ranked[p].score;
        }
        let terms = query_terms(self.core.analyzer(), query);
        let mut explanation = effect_explanation(
            &hit,
            position.map(|p| p + 1),
            Some(&terms),
            options,
            self.clock.now_millis(),
        );
        explanation.retrieved = position.is_some();
        if position.is_none() {
            explanation.lexical_score = None;
            explanation.retrieval_score = None;
        }
        Ok(explanation)
    }

    pub(super) fn explain_region(
        &self,
        query: &str,
        region: u32,
        options: &RecallOptions,
    ) -> Result<Explanation> {
        let found = self
            .core
            .with_experience(|store| {
                let hit = store.region_hit(region);
                let ranked = store.search_regions(query, EXPLAIN_REGIONS);
                (hit, ranked)
            })
            .and_then(|(hit, ranked)| hit.map(|h| (h, ranked)));
        let Some((mut hit, ranked)) = found else {
            return Err(
                MemoryError::NotFound(format!("unknown region {REGION_PREFIX}{region}")).into(),
            );
        };
        let ranked = ranked.map_err(experience_error)?;
        let position = ranked.iter().position(|h| h.region == region);
        if let Some(p) = position {
            hit.score = ranked[p].score;
        }
        let terms = query_terms(self.core.analyzer(), query);
        let mut explanation = region_explanation(
            &hit,
            position.map(|p| p + 1),
            Some(&terms),
            options,
            self.clock.now_millis(),
        );
        explanation.retrieved = position.is_some();
        if position.is_none() {
            explanation.lexical_score = None;
            explanation.retrieval_score = None;
        }
        Ok(explanation)
    }
}

fn region_explanation(
    hit: &RegionHit,
    rank: Option<usize>,
    terms: Option<&super::recall::ExplainTerms>,
    options: &RecallOptions,
    now: Millis,
) -> Explanation {
    Explanation {
        retrieved: true,
        rank,
        lexical_score: Some(hit.score),
        semantic_score: None,
        retrieval_score: Some(hit.score),
        lexical_weight: 1.0,
        semantic_weight: 0.0,
        fusion: "rrf".into(),
        matched_terms: terms.map_or_else(Vec::new, |t| matched_terms(t, &hit.text)),
        recency_factor: 1.0,
        historical_factor: 1.0,
        final_score: None,
        status: FactStatus::Current,
        as_of: options.as_of.unwrap_or(now),
        known_at: options.known_at,
        valid_from: None,
        valid_to: None,
        within_validity: true,
        chain: Vec::new(),
        provenance: Some(
            hit.provenance
                .iter()
                .map(|p| ObservationProvenance {
                    episode: p.episode.clone(),
                    step: p.step,
                    count: p.count,
                })
                .collect(),
        ),
    }
}

fn effect_explanation(
    hit: &EffectHit,
    rank: Option<usize>,
    terms: Option<&super::recall::ExplainTerms>,
    options: &RecallOptions,
    now: Millis,
) -> Explanation {
    let text = hit.view.render();
    let mut e = region_explanation(
        &RegionHit {
            region: 0,
            score: hit.score,
            text,
            breadcrumb: String::new(),
            page_title: hit.view.template.title_template.clone(),
            url_template: hit.view.template.url_template.clone(),
            episodes: 0,
            steps: 0,
            provenance: hit.provenance.clone(),
        },
        rank,
        terms,
        options,
        now,
    );
    e.fusion = "rrf".into();
    e
}

fn effect_result(hit: EffectHit, score: f64, explanation: Option<Explanation>) -> Recalled {
    let result = hit.into_document_result();
    Recalled {
        id: result.document_id,
        text: result.text,
        score,
        status: FactStatus::Current,
        ingest_time: None,
        event_time: None,
        valid_from: None,
        valid_to: None,
        subject: None,
        entities: Vec::new(),
        supersedes: Vec::new(),
        superseded_by: Vec::new(),
        metadata: result.metadata,
        source_uri: None,
        explanation,
    }
}

fn region_result(hit: RegionHit, score: f64, explanation: Option<Explanation>) -> Recalled {
    let result = hit.into_document_result();
    Recalled {
        id: result.document_id,
        text: result.text,
        score,
        status: FactStatus::Current,
        ingest_time: None,
        event_time: None,
        valid_from: None,
        valid_to: None,
        subject: None,
        entities: Vec::new(),
        supersedes: Vec::new(),
        superseded_by: Vec::new(),
        metadata: result.metadata,
        source_uri: None,
        explanation,
    }
}
