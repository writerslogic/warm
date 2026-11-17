// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! The experience core: memory for tree-structured, highly repetitive observations such as
//! agent trajectories over accessibility trees.
//!
//! One observation step is interned into a hash-consed term arena, cut into regions that the
//! existing lexical index searches, and compared with the previous step of its episode to
//! update a table of what each action changes. Equal structure is stored once; every holder
//! of a node is counted, so purging an episode frees exactly what no other episode shares.

mod effects;
pub mod form;
mod regions;
mod snapshot;
pub mod store;
mod terms;
#[cfg(test)]
mod tests;
pub mod tree;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use self::effects::{delta, EffectTable, Frames, Templates};
pub use self::form::{Control, ControlOption};
use self::regions::{Region, RegionStore};
use self::terms::{Sym, TermStore};
use self::tree::{action_label, label, title_template, trunc, url_template};
pub use self::tree::{parse_indented, Action, TreeNode};
use crate::core::documents::{DocumentInput, DocumentResult, MemoryDocumentStore, SearchOptions};

pub use self::snapshot::SNAPSHOT_FILE;

pub(crate) const HARD_MAX_DEPTH: usize = 512;
const INDEX_DIMENSIONS: usize = 256;
const MAX_PROVENANCE: usize = 8;
const FLAG_NO_CHANGE: u8 = 1;
const FLAG_ERROR: u8 = 2;

/// Bounds on what one observation may contain; trees come from outside the process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExperienceConfig {
    /// Levels of nesting, the root counting as one. At most 512.
    pub max_depth: usize,
    /// Nodes in one observation tree.
    pub max_nodes: usize,
    /// Bytes in any role, name, props, handle, URL or action string.
    pub max_text_bytes: usize,
    /// Bytes in an episode id.
    pub max_id_bytes: usize,
    /// Bytes in a goal or outcome.
    pub max_goal_bytes: usize,
    /// Steps recorded for one episode.
    pub max_steps: usize,
    /// Bytes of the snapshot payload. A snapshot over this is refused when it is read, so a
    /// write that would take the store past it is refused before it is journaled.
    pub max_snapshot_bytes: u64,
    /// Parse the controls of a step from the accessibility-tree text an observation carries
    /// (`accessibility_tree`) and store them with the step. Off, the text is dropped unread.
    pub form_state: bool,
    /// Bytes of the accessibility-tree text of one observation. Its lines longer than
    /// `max_text_bytes` are skipped by the form-state parser.
    pub max_page_bytes: usize,
}

impl Default for ExperienceConfig {
    fn default() -> Self {
        Self {
            max_depth: 128,
            max_nodes: 200_000,
            max_text_bytes: 16 * 1024,
            max_id_bytes: 256,
            max_goal_bytes: 64 * 1024,
            max_steps: 1 << 20,
            max_snapshot_bytes: snapshot::MAX_SNAPSHOT_BYTES,
            form_state: true,
            max_page_bytes: 4 << 20,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExperienceError {
    #[error("{what} is {got}, over the limit of {limit}")]
    Limit {
        what: &'static str,
        limit: usize,
        got: usize,
    },
    #[error("invalid {0}")]
    Invalid(&'static str),
    #[error("step {step} of episode {episode:?} precedes the last step {last}")]
    OutOfOrder {
        episode: String,
        step: u32,
        last: u32,
    },
    #[error("step {step} of episode {episode:?} was already ingested with different content")]
    Conflict { episode: String, step: u32 },
    #[error("experience quota of {0} units would be exceeded")]
    Quota(u64),
    #[error("lexical index: {0}")]
    Index(String),
    #[error("snapshot: {0}")]
    Snapshot(String),
    #[error("io: {0}")]
    Io(String),
}

pub type Result<T> = std::result::Result<T, ExperienceError>;

/// One observation step to ingest.
#[derive(Clone, Copy, Debug)]
pub struct Observation<'a> {
    pub episode: &'a str,
    /// Position in the episode; strictly increasing across new steps.
    pub step: u32,
    pub tree: &'a TreeNode,
    pub url: Option<&'a str>,
    /// The action that led to this observation; absent for the first one.
    pub action: Option<&'a Action>,
}

/// An owned observation, the form that arrives as JSON.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnedObservation {
    pub episode: String,
    pub step: u32,
    pub tree: TreeNode,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub action: Option<Action>,
    /// What the episode is for. Setting it, or `outcome`, describes the episode as
    /// [`Experience::describe_episode`] does; the other one keeps its stored value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// The accessibility-tree text of the step, one `[id] role 'name' props` node per line. With
    /// [`ExperienceConfig::form_state`] the controls are parsed from it and stored with the
    /// step; the text itself is not stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessibility_tree: Option<String>,
}

impl OwnedObservation {
    pub fn as_observation(&self) -> Observation<'_> {
        Observation {
            episode: &self.episode,
            step: self.step,
            tree: &self.tree,
            url: self.url.as_deref(),
            action: self.action.as_ref(),
        }
    }
}

/// The weight a result at 1-based `rank` carries when ranked lists are merged by reciprocal rank
/// fusion: `1 / (60 + rank)`. The memory API and the MCP server merge region hits with
/// document hits by this rule, since the two lists come from separate lexical indexes.
pub fn rank_fusion_score(rank: usize) -> f64 {
    1.0 / (60.0 + rank as f64)
}

/// Recognizes a tree-structured observation by its shape: an object holding an `episode`
/// string, a `step` number and a `tree` object. Anything else is an ordinary memory.
pub fn detect_observation(value: &Value) -> Option<OwnedObservation> {
    is_observation_shape(value)
        .then(|| serde_json::from_value(value.clone()).ok())
        .flatten()
}

/// Whether the value has the shape of an observation: an object holding an `episode` string, a
/// `step` number and a `tree` object. It does not check the rest of the fields.
pub fn is_observation_shape(value: &Value) -> bool {
    value.as_object().is_some_and(is_observation_object)
}

/// [`is_observation_shape`] for an object already in hand.
pub fn is_observation_object(o: &serde_json::Map<String, Value>) -> bool {
    o.get("episode").is_some_and(Value::is_string)
        && o.get("step").is_some_and(Value::is_u64)
        && o.get("tree").is_some_and(Value::is_object)
}

/// Checks the bounds of a JSON observation (depth, node count, string and id bytes) by walking
/// the value, so an oversized tree is refused before it is copied into a [`TreeNode`].
pub fn check_observation_value(value: &Value, cfg: &ExperienceConfig) -> Result<()> {
    let depth_limit = cfg.max_depth.clamp(1, HARD_MAX_DEPTH);
    let text = |v: Option<&Value>, what: &'static str, limit: usize| -> Result<()> {
        match v {
            Some(Value::String(s)) => check_limit(what, limit, s.len()),
            _ => Ok(()),
        }
    };
    let Some(o) = value.as_object() else {
        return Err(ExperienceError::Invalid("observation"));
    };
    if matches!(o.get("episode"), Some(Value::String(e)) if e.is_empty() || e.contains('\0')) {
        return Err(ExperienceError::Invalid("episode id"));
    }
    text(o.get("episode"), "episode id bytes", cfg.max_id_bytes)?;
    text(o.get("url"), "url bytes", cfg.max_text_bytes)?;
    text(o.get("goal"), "goal bytes", cfg.max_goal_bytes)?;
    text(o.get("outcome"), "outcome bytes", cfg.max_goal_bytes)?;
    text(
        o.get("accessibilityTree"),
        "accessibility tree bytes",
        cfg.max_page_bytes,
    )?;
    if let Some(Value::Object(a)) = o.get("action") {
        for v in a.values() {
            text(Some(v), "action bytes", cfg.max_text_bytes)?;
        }
    }
    let (mut nodes, mut stack) = (0usize, Vec::new());
    if let Some(t) = o.get("tree") {
        stack.push((t, 1usize));
    }
    while let Some((n, depth)) = stack.pop() {
        let Value::Object(m) = n else {
            return Err(ExperienceError::Invalid("tree node"));
        };
        nodes += 1;
        check_limit("tree nodes", cfg.max_nodes, nodes)?;
        check_limit("tree depth", depth_limit, depth)?;
        if m.get("role")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return Err(ExperienceError::Invalid("empty role"));
        }
        for key in ["role", "name", "props", "handle"] {
            text(m.get(key), "string bytes", cfg.max_text_bytes)?;
        }
        match m.get("children") {
            Some(Value::Array(children)) => {
                // Bounds the stack before it is filled.
                check_limit("tree nodes", cfg.max_nodes, nodes + children.len())?;
                stack.extend(children.iter().map(|c| (c, depth + 1)));
            }
            Some(Value::Null) | None => {}
            Some(_) => return Err(ExperienceError::Invalid("tree children")),
        }
    }
    Ok(())
}

fn check_limit(what: &'static str, limit: usize, got: usize) -> Result<()> {
    Experience::limit(what, limit, got)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ingested {
    Applied,
    /// The same step with the same content was ingested before; nothing changed.
    Duplicate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    pub episode: String,
    pub step: u32,
    /// How many times the region occurs in that step.
    pub count: u32,
}

const EFFECT_INDEX_TOP: usize = 14;

fn effect_doc_id(template: u32, action: &str) -> String {
    format!("x{template}:{action}")
}

fn parse_effect_doc_id(id: &str) -> Option<(u32, String)> {
    let (t, a) = id.strip_prefix('x')?.split_once(':')?;
    Some((t.parse().ok()?, a.to_string()))
}

/// An action-effect row found by search. `id` is the stable index id, `x{template}:{action}`.
#[derive(Clone, Debug)]
pub struct EffectHit {
    pub id: String,
    pub score: f64,
    pub view: EffectView,
    /// The first transitions by episode and step that produced this row, at most eight.
    pub provenance: Vec<Provenance>,
}

impl EffectHit {
    /// The hit as an ordinary document result, so a recall can merge it with document hits.
    pub fn into_document_result(self) -> DocumentResult {
        let text = self.view.render();
        let end = text.len() as u32;
        let prov: Vec<Value> = self
            .provenance
            .iter()
            .map(|p| json!({"episode": p.episode, "step": p.step, "count": p.count}))
            .collect();
        let id = format!("experience:{}", self.id);
        DocumentResult {
            id: id.clone(),
            document_id: id,
            source_uri: None,
            version: String::new(),
            metadata: json!({
                "kind": "experience-effect",
                "pageTitle": self.view.template.title_template,
                "urlTemplate": self.view.template.url_template,
                "action": self.view.action,
                "count": self.view.count,
                "provenance": prov,
            }),
            text: Some(text),
            start_byte: 0,
            end_byte: end,
            score: self.score,
            lexical_score: self.score,
            semantic_score: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RegionHit {
    pub region: u32,
    pub score: f64,
    pub text: String,
    pub breadcrumb: String,
    pub page_title: String,
    pub url_template: String,
    pub episodes: usize,
    pub steps: usize,
    /// The first occurrences by episode and step, at most eight.
    pub provenance: Vec<Provenance>,
}

impl RegionHit {
    /// The hit as an ordinary document result, so a recall can merge it with document hits.
    pub fn into_document_result(self) -> DocumentResult {
        let end = self.text.len() as u32;
        let prov: Vec<Value> = self
            .provenance
            .iter()
            .map(|p| json!({"episode": p.episode, "step": p.step, "count": p.count}))
            .collect();
        DocumentResult {
            id: format!("experience:r{}", self.region),
            document_id: format!("experience:r{}", self.region),
            source_uri: None,
            version: String::new(),
            metadata: json!({
                "kind": "experience-region",
                "breadcrumb": self.breadcrumb,
                "pageTitle": self.page_title,
                "urlTemplate": self.url_template,
                "episodes": self.episodes,
                "steps": self.steps,
                "provenance": prov,
            }),
            text: Some(self.text),
            start_byte: 0,
            end_byte: end,
            score: self.score,
            lexical_score: self.score,
            semantic_score: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateInfo {
    pub id: u32,
    pub url_template: String,
    pub title_template: String,
    pub observations: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectView {
    pub template: TemplateInfo,
    pub action: String,
    pub count: u32,
    pub self_loops: u32,
    pub added: Vec<(String, u32)>,
    pub removed: Vec<(String, u32)>,
    pub next: Vec<(TemplateInfo, u32)>,
    pub errors: Vec<(String, u32)>,
}

impl EffectView {
    /// One line a reader can use as evidence.
    pub fn render(&self) -> String {
        let mut x = format!(
            "On page '{}': {} (x{}) -> ",
            self.template.title_template, self.action, self.count
        );
        if self.self_loops == self.count {
            x.push_str("no visible change (self-loop)");
        } else {
            let join = |v: &[(String, u32)], n: usize| {
                v.iter()
                    .take(n)
                    .map(|p| p.0.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            };
            x.push_str(&format!("now shown: {}", join(&self.added, 14)));
            if !self.removed.is_empty() {
                x.push_str(&format!(" | gone: {}", join(&self.removed, 6)));
            }
            let next = self.next.first().map_or("", |n| &n.0.title_template);
            x.push_str(&format!(" | next page: '{next}'"));
        }
        if !self.errors.is_empty() {
            let e: Vec<&str> = self.errors.iter().take(2).map(|p| p.0.as_str()).collect();
            x.push_str(&format!(" | ERROR text: {}", e.join("; ")));
        }
        if self.self_loops > 0 && self.self_loops < self.count {
            x.push_str(&format!(
                " | nondeterministic: {} of {} no change",
                self.self_loops, self.count
            ));
        }
        x
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathEntry {
    pub step: u32,
    pub action: Option<String>,
    pub template: TemplateInfo,
    pub no_change: bool,
    pub error: bool,
}

#[derive(Clone, Debug)]
pub struct EpisodeView {
    pub episode: String,
    pub goal: String,
    pub outcome: String,
    pub path: Vec<PathEntry>,
}

#[derive(Clone, Debug)]
pub struct EpisodeHit {
    pub episode: String,
    pub goal: String,
    pub outcome: String,
    pub score: f64,
    pub steps: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PurgeReport {
    pub steps: usize,
    pub regions_freed: usize,
    pub nodes_freed: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExperienceStats {
    pub episodes: usize,
    pub steps: usize,
    pub nodes: usize,
    pub structural: usize,
    pub regions: usize,
    pub region_occurrences: u64,
    pub templates: usize,
    pub frames: usize,
    pub effect_rows: usize,
    pub symbols: usize,
}

type Contribution1 = (String, String, String, String);
type Contribution = (Vec<Box<[u32]>>, Option<Contribution1>);
type Rederived = (Option<Contribution1>, Vec<Box<[u32]>>);

pub(crate) struct StepRec {
    pub step: u32,
    pub root: u32,
    pub template: u32,
    pub frame: u32,
    pub label: Option<Sym>,
    pub flags: u8,
    pub regions: Vec<(u32, u32)>,
    /// The form controls of the step, from its accessibility-tree text.
    pub controls: Box<[Control]>,
}

pub(crate) struct Episode {
    pub key: String,
    pub goal: String,
    pub outcome: String,
    pub steps: Vec<StepRec>,
    /// Handle to element label for the latest observation; not persisted.
    pub handles: HashMap<String, String>,
}

pub struct Experience {
    cfg: ExperienceConfig,
    terms: TermStore,
    regions: RegionStore,
    templates: Templates,
    frames: Frames,
    effects: EffectTable,
    episodes: Vec<Option<Episode>>,
    free_episodes: Vec<u32>,
    episode_map: HashMap<String, u32>,
    region_docs: MemoryDocumentStore,
    effect_docs: MemoryDocumentStore,
    episode_docs: MemoryDocumentStore,
    dirty: BTreeSet<u32>,
    /// Steps stored over all episodes, and the region entries they carry.
    step_count: usize,
    step_region_count: usize,
    /// Controls stored over all steps, and the bytes of text they hold.
    step_controls: usize,
    step_control_bytes: usize,
}

fn doc(id: String, text: String) -> DocumentInput {
    DocumentInput {
        id,
        text,
        source_uri: None,
        version: None,
        metadata: None,
        chunk_words: Some(4096),
        overlap_words: Some(0),
        store_text: Some(false),
        embeddings: None,
        title_weight: None,
    }
}

fn index_err(e: impl std::fmt::Display) -> ExperienceError {
    ExperienceError::Index(e.to_string())
}

fn collect_handles(t: &TreeNode, out: &mut HashMap<String, String>) {
    let mut stack = vec![t];
    while let Some(n) = stack.pop() {
        if let Some(h) = &n.handle {
            out.insert(h.clone(), label(&n.role, &n.name));
        }
        stack.extend(n.children.iter().rev());
    }
}

impl Default for Experience {
    fn default() -> Self {
        Self::new(ExperienceConfig::default())
    }
}

impl Experience {
    pub fn new(cfg: ExperienceConfig) -> Self {
        let mut cfg = cfg;
        cfg.max_depth = cfg.max_depth.clamp(1, HARD_MAX_DEPTH);
        cfg.max_snapshot_bytes = cfg.max_snapshot_bytes.min(snapshot::MAX_SNAPSHOT_BYTES);
        Self {
            cfg,
            terms: TermStore::default(),
            regions: RegionStore::default(),
            templates: Templates::default(),
            frames: Frames::default(),
            effects: EffectTable::default(),
            episodes: Vec::new(),
            free_episodes: Vec::new(),
            episode_map: HashMap::new(),
            effect_docs: MemoryDocumentStore::new(INDEX_DIMENSIONS, None).expect("effect index"),
            region_docs: MemoryDocumentStore::new(INDEX_DIMENSIONS, None)
                .expect("index dimensions are in range"),
            episode_docs: MemoryDocumentStore::new(INDEX_DIMENSIONS, None)
                .expect("index dimensions are in range"),
            dirty: BTreeSet::new(),
            step_count: 0,
            step_region_count: 0,
            step_controls: 0,
            step_control_bytes: 0,
        }
    }

    pub fn config(&self) -> &ExperienceConfig {
        &self.cfg
    }

    fn limit(what: &'static str, limit: usize, got: usize) -> Result<()> {
        if got > limit {
            Err(ExperienceError::Limit { what, limit, got })
        } else {
            Ok(())
        }
    }

    fn validate(&self, o: &Observation<'_>) -> Result<()> {
        let c = &self.cfg;
        if o.episode.is_empty() || o.episode.contains('\0') {
            return Err(ExperienceError::Invalid("episode id"));
        }
        Self::limit("episode id bytes", c.max_id_bytes, o.episode.len())?;
        Self::limit("url bytes", c.max_text_bytes, o.url.map_or(0, str::len))?;
        if let Some(a) = o.action {
            let (Action::Label(t) | Action::Call(t)) = a;
            Self::limit("action bytes", c.max_text_bytes, t.len())?;
        }
        let (mut nodes, mut stack) = (0usize, vec![(o.tree, 1usize)]);
        while let Some((n, depth)) = stack.pop() {
            nodes += 1;
            Self::limit("tree nodes", c.max_nodes, nodes)?;
            Self::limit("tree depth", c.max_depth, depth)?;
            if n.role.is_empty() {
                return Err(ExperienceError::Invalid("empty role"));
            }
            for s in [&n.role, &n.name, &n.props] {
                Self::limit("string bytes", c.max_text_bytes, s.len())?;
            }
            Self::limit(
                "handle bytes",
                c.max_text_bytes,
                n.handle.as_ref().map_or(0, String::len),
            )?;
            stack.extend(n.children.iter().map(|k| (k, depth + 1)));
        }
        Ok(())
    }

    /// Whether the episode is stored.
    pub fn has_episode(&self, id: &str) -> bool {
        self.episode_map.contains_key(id)
    }

    /// The goal and outcome of an episode.
    pub fn episode_description(&self, id: &str) -> Option<(&str, &str)> {
        let e = self.ep(self.episode_slot(id)?);
        Some((&e.goal, &e.outcome))
    }

    fn episode_slot(&self, key: &str) -> Option<u32> {
        self.episode_map.get(key).copied()
    }

    fn ep(&self, id: u32) -> &Episode {
        self.episodes[id as usize].as_ref().expect("live episode")
    }

    fn new_episode(&mut self, key: &str) -> u32 {
        let e = Episode {
            key: key.to_string(),
            goal: String::new(),
            outcome: String::new(),
            steps: Vec::new(),
            handles: HashMap::new(),
        };
        let id = match self.free_episodes.pop() {
            Some(i) => {
                self.episodes[i as usize] = Some(e);
                i
            }
            None => {
                self.episodes.push(Some(e));
                (self.episodes.len() - 1) as u32
            }
        };
        self.episode_map.insert(key.to_string(), id);
        id
    }

    /// Sets the goal and outcome of an episode, creating it if needed.
    pub fn describe_episode(&mut self, episode: &str, goal: &str, outcome: &str) -> Result<()> {
        if episode.is_empty() || episode.contains('\0') {
            return Err(ExperienceError::Invalid("episode id"));
        }
        Self::limit("episode id bytes", self.cfg.max_id_bytes, episode.len())?;
        Self::limit("goal bytes", self.cfg.max_goal_bytes, goal.len())?;
        Self::limit("outcome bytes", self.cfg.max_goal_bytes, outcome.len())?;
        let id = self
            .episode_slot(episode)
            .unwrap_or_else(|| self.new_episode(episode));
        let e = self.episodes[id as usize].as_mut().expect("live episode");
        e.goal = goal.to_string();
        e.outcome = outcome.to_string();
        self.dirty.insert(id);
        Ok(())
    }

    /// Ingests one observation step. Retrying a step with identical content changes nothing.
    pub fn ingest(&mut self, o: &Observation<'_>) -> Result<Ingested> {
        self.ingest_with_controls(o, Vec::new())
    }

    /// The form controls of an accessibility-tree text ([`form::extract_controls`]), bounded by
    /// this store's limits; none when [`ExperienceConfig::form_state`] is off.
    pub fn extract_controls(&self, accessibility_tree: &str) -> Result<Vec<Control>> {
        Self::limit(
            "accessibility tree bytes",
            self.cfg.max_page_bytes,
            accessibility_tree.len(),
        )?;
        if !self.cfg.form_state {
            return Ok(Vec::new());
        }
        Ok(form::extract_controls(
            accessibility_tree,
            self.cfg.max_text_bytes,
        ))
    }

    fn validate_controls(&self, controls: &[Control]) -> Result<()> {
        Self::limit("step controls", form::MAX_CONTROLS, controls.len())?;
        for c in controls {
            Self::limit("control options", form::MAX_OPTIONS, c.options.len())?;
            Self::limit("string bytes", self.cfg.max_text_bytes, c.longest_string())?;
        }
        Ok(())
    }

    /// [`Self::ingest`] storing `controls` with the step. The controls are part of the step's
    /// content: the same step again with other controls is a conflict.
    pub fn ingest_with_controls(
        &mut self,
        o: &Observation<'_>,
        controls: Vec<Control>,
    ) -> Result<Ingested> {
        self.validate(o)?;
        self.validate_controls(&controls)?;
        let eid = self.episode_slot(o.episode);
        if let Some(id) = eid {
            let e = self.ep(id);
            Self::limit("episode steps", self.cfg.max_steps, e.steps.len() + 1).or_else(|err| {
                if e.steps.binary_search_by_key(&o.step, |s| s.step).is_ok() {
                    Ok(())
                } else {
                    Err(err)
                }
            })?;
            if let Some(last) = e.steps.last() {
                let seen = e.steps.binary_search_by_key(&o.step, |s| s.step).is_ok();
                if o.step < last.step && !seen {
                    return Err(ExperienceError::OutOfOrder {
                        episode: o.episode.to_string(),
                        step: o.step,
                        last: last.step,
                    });
                }
            }
        }

        let root = self.terms.intern(o.tree);
        let template = self.templates.acquire(
            &mut self.terms.syms,
            &url_template(o.url.unwrap_or("")),
            &title_template(&o.tree.name),
        );
        let handles = eid.map(|id| &self.ep(id).handles);
        let label_text = o.action.map(|a| match a {
            Action::Label(l) => l.clone(),
            Action::Call(c) => action_label(c, &|h| handles.and_then(|m| m.get(h).cloned())),
        });
        let label_sym = label_text.map(|t| self.terms.syms.acquire(&t));

        if let Some(id) = eid {
            let e = self.ep(id);
            if let Ok(i) = e.steps.binary_search_by_key(&o.step, |s| s.step) {
                let s = &e.steps[i];
                let same = s.root == root
                    && s.template == template
                    && s.label == label_sym
                    && *s.controls == *controls;
                self.terms.release(root);
                self.templates.release(&mut self.terms.syms, template);
                if let Some(l) = label_sym {
                    self.terms.syms.release(l);
                }
                return if same {
                    Ok(Ingested::Duplicate)
                } else {
                    Err(ExperienceError::Conflict {
                        episode: o.episode.to_string(),
                        step: o.step,
                    })
                };
            }
        }

        let eid = eid.unwrap_or_else(|| self.new_episode(o.episode));
        let mut lines = BTreeSet::new();
        regions::flat_lines(&self.terms, root, &mut lines);
        let mut frame: Vec<Sym> = lines.iter().map(|l| self.terms.syms.acquire(l)).collect();
        frame.sort_unstable();
        let frame_id = self
            .frames
            .acquire(&mut self.terms.syms, frame.into_boxed_slice());
        let frame_lines: Box<[Sym]> = self.frames.get(frame_id).expect("live frame").lines.clone();
        self.templates
            .observe(&mut self.terms.syms, template, &frame_lines, true);

        let mut flags = 0u8;
        if let (Some(prev), Some(l)) = (self.ep(eid).steps.last(), label_sym) {
            let prev_lines = &self.frames.get(prev.frame).expect("live frame").lines;
            let d = delta(&self.terms.syms, prev_lines, &frame_lines);
            if d.no_change {
                flags |= FLAG_NO_CHANGE;
            }
            if !d.errors.is_empty() {
                flags |= FLAG_ERROR;
            }
            let prev_template = prev.template;
            let key = (prev_template, l, template);
            self.effects
                .apply(&mut self.terms.syms, &mut self.templates, key, &d, true);
            let action = self.terms.syms.get(l).to_string();
            self.sync_effect_doc(prev_template, &action);
        }

        let title = title_template(&o.tree.name);
        let url_t = url_template(o.url.unwrap_or(""));
        let mut counts: BTreeMap<u32, u32> = BTreeMap::new();
        for b in regions::cut(&self.terms, root) {
            let key: Box<[u32]> = b.cids.iter().map(|&c| self.terms.node(c).sid).collect();
            let rid = match self.regions.find(&key) {
                Some(r) => r,
                None => {
                    let text = regions::render_batch(&self.terms, &b.cids);
                    if !regions::text_is_indexable(&text) {
                        continue;
                    }
                    for &sid in key.iter() {
                        self.terms.bump_sid(sid);
                    }
                    let region = Region {
                        key,
                        breadcrumb: b.breadcrumb.clone(),
                        title: title.clone(),
                        url: url_t.clone(),
                        text,
                        variants: Vec::new(),
                        values: HashMap::new(),
                        postings: Vec::new(),
                    };
                    let index_text = region.index_text();
                    let rid = self.regions.insert(region);
                    self.region_docs
                        .memorize(doc(format!("r{rid}"), index_text))
                        .map_err(index_err)?;
                    rid
                }
            };
            self.observe_variant(rid, &b.cids);
            *counts.entry(rid).or_insert(0) += 1;
        }
        for (&rid, &n) in &counts {
            if let Some(Some(r)) = self.regions.slots.get_mut(rid as usize) {
                r.add_posting(eid, o.step, n);
            }
        }

        let mut handle_map = HashMap::new();
        collect_handles(o.tree, &mut handle_map);
        let e = self.episodes[eid as usize].as_mut().expect("live episode");
        e.handles = handle_map;
        let regions: Vec<(u32, u32)> = counts.into_iter().collect();
        self.step_count += 1;
        self.step_region_count += regions.len();
        self.step_controls += controls.len();
        self.step_control_bytes += controls.iter().map(Control::text_bytes).sum::<usize>();
        e.steps.push(StepRec {
            step: o.step,
            root,
            template,
            frame: frame_id,
            label: label_sym,
            flags,
            regions,
            controls: controls.into_boxed_slice(),
        });
        self.dirty.insert(eid);
        Ok(Ingested::Applied)
    }

    fn observe_variant(&mut self, rid: u32, cids: &[u32]) {
        let Some(Some(r)) = self.regions.slots.get_mut(rid as usize) else {
            return;
        };
        if r.variants.len() >= regions::MAX_VARIANTS || r.variants.iter().any(|v| **v == *cids) {
            return;
        }
        let mut vals = Vec::new();
        for &c in cids {
            self.terms.bump(c);
            regions::value_lines(&self.terms, c, &mut vals);
        }
        for v in vals {
            *r.values.entry(trunc(&v, 160).to_string()).or_insert(0) += 1;
        }
        r.variants.push(cids.into());
    }

    pub fn stats(&self) -> ExperienceStats {
        ExperienceStats {
            episodes: self.episode_map.len(),
            steps: self.episodes.iter().flatten().map(|e| e.steps.len()).sum(),
            nodes: self.terms.live_nodes(),
            structural: self.terms.live_structs(),
            regions: self.regions.live(),
            region_occurrences: self
                .regions
                .slots
                .iter()
                .flatten()
                .map(Region::occurrences)
                .sum(),
            templates: self.templates.live(),
            frames: self.frames.live(),
            effect_rows: self.effects.rows.len(),
            symbols: self.terms.syms.live(),
        }
    }

    fn info(&self, id: u32) -> TemplateInfo {
        let t = self.templates.get(id).expect("live template");
        TemplateInfo {
            id,
            url_template: self.terms.syms.get(t.url).to_string(),
            title_template: self.terms.syms.get(t.title).to_string(),
            observations: t.obs,
        }
    }

    /// Regions ranked by the lexical index, each with where it occurred.
    pub fn search_regions(&self, query: &str, k: usize) -> Result<Vec<RegionHit>> {
        if query.trim().is_empty() || k == 0 {
            return Ok(Vec::new());
        }
        let opts = SearchOptions {
            k: Some(k.min(1000) as u32),
            ..Default::default()
        };
        let found = self.region_docs.search(query, &opts).map_err(index_err)?;
        let mut hits = Vec::new();
        for d in found {
            let Some(rid) = d
                .document_id
                .strip_prefix('r')
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            let Some(r) = self.regions.get(rid) else {
                continue;
            };
            let provenance = r
                .postings
                .iter()
                .take(MAX_PROVENANCE)
                .map(|p| Provenance {
                    episode: self.ep(p.0).key.clone(),
                    step: p.1,
                    count: p.2,
                })
                .collect();
            hits.push(RegionHit {
                region: rid,
                score: d.score,
                text: r.unit_text(),
                breadcrumb: r.breadcrumb.clone(),
                page_title: r.title.clone(),
                url_template: r.url.clone(),
                episodes: r.episodes(),
                steps: r.postings.len(),
                provenance,
            });
        }
        Ok(hits)
    }

    /// One region by id, with where it occurred; its score is 0.
    pub fn region_hit(&self, rid: u32) -> Option<RegionHit> {
        let r = self.regions.get(rid)?;
        Some(RegionHit {
            region: rid,
            score: 0.0,
            text: r.unit_text(),
            breadcrumb: r.breadcrumb.clone(),
            page_title: r.title.clone(),
            url_template: r.url.clone(),
            episodes: r.episodes(),
            steps: r.postings.len(),
            provenance: r
                .postings
                .iter()
                .take(MAX_PROVENANCE)
                .map(|p| Provenance {
                    episode: self.ep(p.0).key.clone(),
                    step: p.1,
                    count: p.2,
                })
                .collect(),
        })
    }

    /// Whether any region is stored, that is, whether a search can return anything.
    pub fn has_regions(&self) -> bool {
        self.regions.live() > 0
    }

    /// Whether anything of the experience core is searchable by recall.
    pub fn is_searchable(&self) -> bool {
        self.regions.live() > 0 || !self.effects.rows.is_empty()
    }

    /// The template of a page, if any observation of it is stored.
    pub fn template_id(&self, url: Option<&str>, title: &str) -> Option<u32> {
        self.templates.lookup(
            &self.terms.syms,
            &url_template(url.unwrap_or("")),
            &title_template(title),
        )
    }

    pub fn templates(&self) -> Vec<TemplateInfo> {
        let mut v: Vec<TemplateInfo> = (0..self.templates.slots.len() as u32)
            .filter(|&i| self.templates.get(i).is_some())
            .map(|i| self.info(i))
            .collect();
        v.sort_by(|a, b| {
            (&a.title_template, &a.url_template).cmp(&(&b.title_template, &b.url_template))
        });
        v
    }

    /// How many observations of the template showed `line`.
    pub fn line_observations(&self, template: u32, line: &str) -> u32 {
        let (Some(t), Some(s)) = (self.templates.get(template), self.terms.syms.lookup(line))
        else {
            return 0;
        };
        t.inventory.get(&s).copied().unwrap_or(0)
    }

    /// What actions on `template` change; one action when `action` is given. Lists are
    /// ordered by count and then text, each cut to `top`.
    pub fn effects(&self, template: u32, action: Option<&str>, top: usize) -> Vec<EffectView> {
        let syms = &self.terms.syms;
        let mut out: Vec<EffectView> = self
            .effects
            .rows
            .iter()
            .filter(|((t, l), _)| *t == template && action.is_none_or(|a| syms.get(*l) == a))
            .map(|(&key, r)| self.effect_view(key, r, top))
            .collect();
        out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.action.cmp(&b.action)));
        out
    }

    fn effect_view(&self, (t, l): (u32, Sym), r: &effects::EffectRow, top: usize) -> EffectView {
        let syms = &self.terms.syms;
        let ranked = |m: &HashMap<Sym, u32>| {
            let mut v: Vec<(&str, u32)> = m.iter().map(|(&s, &c)| (syms.get(s), c)).collect();
            let order = |a: &(&str, u32), b: &(&str, u32)| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0));
            if top == 0 {
                return Vec::new();
            }
            // Only the first `top` are kept, so the rest are never sorted or copied.
            if v.len() > top {
                v.select_nth_unstable_by(top - 1, order);
                v.truncate(top);
            }
            v.sort_by(order);
            v.into_iter().map(|(t, c)| (t.to_string(), c)).collect()
        };
        let mut next: Vec<(TemplateInfo, u32)> =
            r.nxt.iter().map(|(&n, &c)| (self.info(n), c)).collect();
        next.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.0.title_template.cmp(&b.0.title_template))
        });
        EffectView {
            template: self.info(t),
            action: syms.get(l).to_string(),
            count: r.n,
            self_loops: r.loops,
            added: ranked(&r.add),
            removed: ranked(&r.rem),
            next,
            errors: ranked(&r.err),
        }
    }

    /// Brings the effect index entry for (`template`, `action`) in line with the table.
    fn sync_effect_doc(&mut self, template: u32, action: &str) {
        let id = effect_doc_id(template, action);
        let row = self
            .terms
            .syms
            .lookup(action)
            .and_then(|l| self.effects.rows.get(&(template, l)).map(|r| (l, r)));
        match row {
            Some((l, r)) => {
                let text = self
                    .effect_view((template, l), r, EFFECT_INDEX_TOP)
                    .render();
                if let Err(e) = self.effect_docs.memorize(doc(id, text)) {
                    debug_assert!(false, "effect index: {e}");
                }
            }
            None => {
                self.effect_docs.delete(&id);
            }
        }
    }

    pub(crate) fn rebuild_effect_docs(&mut self) -> Result<()> {
        let mut docs = MemoryDocumentStore::new(INDEX_DIMENSIONS, None).map_err(index_err)?;
        for (&key, r) in &self.effects.rows {
            let v = self.effect_view(key, r, EFFECT_INDEX_TOP);
            docs.memorize(doc(effect_doc_id(key.0, &v.action), v.render()))
                .map_err(index_err)?;
        }
        self.effect_docs = docs;
        Ok(())
    }

    /// What counts against a tenant quota: one unit per episode, step, region and effect row.
    /// A step counts because regions are keyed by structure, so steps that differ only in
    /// their props add no region yet each stores them.
    pub fn quota_units(&self) -> u64 {
        (self.episode_map.len() + self.step_count + self.regions.live() + self.effects.rows.len())
            as u64
    }

    /// Whether `step` of `episode` is already stored, so ingesting it again adds nothing.
    pub fn has_step(&self, episode: &str, step: u32) -> bool {
        self.episode_slot(episode).is_some_and(|id| {
            self.ep(id)
                .steps
                .binary_search_by_key(&step, |s| s.step)
                .is_ok()
        })
    }

    /// An over-estimate of the snapshot payload, from counters kept as the store changes.
    fn snapshot_estimate(&self) -> u64 {
        const NODE: usize = 64;
        const SYMBOL: usize = 16;
        const STEP: usize = 32;
        const STEP_REGION: usize = 12;
        const REGION: usize = 2048;
        const CONTROL: usize = 48;
        (self.step_control_bytes
            + CONTROL * self.step_controls
            + self.terms.syms.live_bytes()
            + SYMBOL * self.terms.syms.live()
            + NODE * self.terms.live_nodes()
            + STEP * self.step_count
            + STEP_REGION * self.step_region_count
            + REGION * self.regions.live()) as u64
    }

    /// Fails when the snapshot of the current state would exceed the snapshot limit. Cheap while
    /// the estimate is well under the limit; near it, measures the real size.
    pub fn check_snapshot_size(&self) -> Result<()> {
        if self.snapshot_estimate() < self.cfg.max_snapshot_bytes / 4 * 3 {
            return Ok(());
        }
        self.to_bytes_at(u64::MAX).map(drop)
    }

    pub fn has_effects(&self) -> bool {
        !self.effects.rows.is_empty()
    }

    /// Action effects ranked by the lexical index over their rendered rows.
    pub fn search_effects(&self, query: &str, k: usize) -> Result<Vec<EffectHit>> {
        if query.trim().is_empty() || k == 0 || self.effects.rows.is_empty() {
            return Ok(Vec::new());
        }
        let opts = SearchOptions {
            k: Some(k.min(1000) as u32),
            ..Default::default()
        };
        let found = self.effect_docs.search(query, &opts).map_err(index_err)?;
        let mut hits: Vec<EffectHit> = found
            .into_iter()
            .filter_map(|d| {
                let (template, action) = parse_effect_doc_id(&d.document_id)?;
                let view = self.effect_view_of(template, &action)?;
                Some(EffectHit {
                    id: d.document_id,
                    score: d.score,
                    view,
                    provenance: Vec::new(),
                })
            })
            .collect();
        self.fill_effect_provenance(&mut hits);
        Ok(hits)
    }

    /// The row for (`template`, `action`), found by key.
    fn effect_view_of(&self, template: u32, action: &str) -> Option<EffectView> {
        let l = self.terms.syms.lookup(action)?;
        let r = self.effects.rows.get(&(template, l))?;
        Some(self.effect_view((template, l), r, EFFECT_INDEX_TOP))
    }

    /// One effect row by its index id, with score 0.
    pub fn effect_hit(&self, id: &str) -> Option<EffectHit> {
        let (template, action) = parse_effect_doc_id(id)?;
        let view = self.effect_view_of(template, &action)?;
        let mut hits = vec![EffectHit {
            id: id.to_string(),
            score: 0.0,
            view,
            provenance: Vec::new(),
        }];
        self.fill_effect_provenance(&mut hits);
        hits.pop()
    }

    /// Fills the first transitions of each hit's row in one pass over the stored steps, however
    /// many hits there are.
    fn fill_effect_provenance(&self, hits: &mut [EffectHit]) {
        let mut by_key: HashMap<(u32, Sym), usize> = HashMap::new();
        for (i, h) in hits.iter().enumerate() {
            let Some((t, a)) = parse_effect_doc_id(&h.id) else {
                continue;
            };
            if let Some(l) = self.terms.syms.lookup(&a) {
                by_key.insert((t, l), i);
            }
        }
        let mut open = by_key.len();
        'scan: for e in self.episodes.iter().flatten() {
            for w in e.steps.windows(2) {
                let Some(l) = w[1].label else { continue };
                let Some(&i) = by_key.get(&(w[0].template, l)) else {
                    continue;
                };
                let p = &mut hits[i].provenance;
                if p.len() < MAX_PROVENANCE {
                    p.push(Provenance {
                        episode: e.key.clone(),
                        step: w[1].step,
                        count: 1,
                    });
                    if p.len() == MAX_PROVENANCE {
                        open -= 1;
                        if open == 0 {
                            break 'scan;
                        }
                    }
                }
            }
        }
    }

    /// Episodes whose goal, outcome or action path matches the text.
    pub fn list_episodes(&mut self, goal_text: &str, k: usize) -> Result<Vec<EpisodeHit>> {
        self.refresh_episode_index()?;
        if goal_text.trim().is_empty() || k == 0 {
            return Ok(Vec::new());
        }
        let opts = SearchOptions {
            k: Some(k.min(1000) as u32),
            ..Default::default()
        };
        let found = self
            .episode_docs
            .search(goal_text, &opts)
            .map_err(index_err)?;
        Ok(found
            .into_iter()
            .filter_map(|d| {
                let id: u32 = d.document_id.strip_prefix('e')?.parse().ok()?;
                let e = self.episodes.get(id as usize)?.as_ref()?;
                Some(EpisodeHit {
                    episode: e.key.clone(),
                    goal: e.goal.clone(),
                    outcome: e.outcome.clone(),
                    score: d.score,
                    steps: e.steps.len(),
                })
            })
            .collect())
    }

    /// The form controls stored with a step; `None` when the step is not stored.
    pub fn step_controls(&self, episode: &str, step: u32) -> Option<&[Control]> {
        let e = self.ep(self.episode_slot(episode)?);
        let i = e.steps.binary_search_by_key(&step, |s| s.step).ok()?;
        Some(&e.steps[i].controls)
    }

    /// The form tables of `states` in order ([`form::render_form_tables`]): each state's
    /// controls, without those already shown for an earlier state of the list. A state that is
    /// not stored or has no controls gives an empty string.
    pub fn form_tables<S: AsRef<str>>(&self, states: &[(S, u32)]) -> Vec<String> {
        form::render_form_tables(
            states
                .iter()
                .map(|(e, s)| self.step_controls(e.as_ref(), *s).unwrap_or(&[])),
        )
    }

    pub fn episode(&self, id: &str) -> Option<EpisodeView> {
        let e = self.ep(self.episode_slot(id)?);
        Some(EpisodeView {
            episode: e.key.clone(),
            goal: e.goal.clone(),
            outcome: e.outcome.clone(),
            path: e
                .steps
                .iter()
                .map(|s| PathEntry {
                    step: s.step,
                    action: s.label.map(|l| self.terms.syms.get(l).to_string()),
                    template: self.info(s.template),
                    no_change: s.flags & FLAG_NO_CHANGE != 0,
                    error: s.flags & FLAG_ERROR != 0,
                })
                .collect(),
        })
    }

    fn episode_text(&self, id: u32) -> String {
        let e = self.ep(id);
        let mut x = format!(
            "Episode {} [{}] goal: {}",
            e.key,
            e.outcome,
            trunc(&e.goal, 300)
        );
        for (i, s) in e.steps.iter().enumerate() {
            let (Some(l), true) = (s.label, i > 0) else {
                continue;
            };
            x.push_str(&format!("\n{}. {}", s.step, self.terms.syms.get(l)));
            if s.flags & FLAG_NO_CHANGE != 0 {
                x.push_str(" (no visible change)");
            }
            if s.flags & FLAG_ERROR != 0 {
                let (Some(p), Some(c)) = (
                    self.frames.get(e.steps[i - 1].frame),
                    self.frames.get(s.frame),
                ) else {
                    continue;
                };
                let d = delta(&self.terms.syms, &p.lines, &c.lines);
                let shown: Vec<&str> = d.errors.iter().take(2).map(String::as_str).collect();
                x.push_str(&format!(" (ERROR text: {})", shown.join("; ")));
            }
        }
        if let Some(s) = e.steps.last() {
            x.push_str(&format!(
                "\nended on: {}",
                self.info(s.template).title_template
            ));
        }
        x
    }

    fn refresh_episode_index(&mut self) -> Result<()> {
        for id in std::mem::take(&mut self.dirty) {
            if self.episodes.get(id as usize).is_some_and(Option::is_some) {
                let text = self.episode_text(id);
                self.episode_docs
                    .memorize(doc(format!("e{id}"), text))
                    .map_err(index_err)?;
            }
        }
        Ok(())
    }

    fn template_strings(&self, template: u32) -> (String, String) {
        let t = self.templates.get(template).expect("live template");
        (
            self.terms.syms.get(t.title).to_string(),
            self.terms.syms.get(t.url).to_string(),
        )
    }

    /// For each of `rids` that the episode touched: the content-id runs it showed and the
    /// (breadcrumb, page title, url template, text) of its first occurrence.
    fn contributions(&self, e: &Episode, rids: &BTreeSet<u32>) -> HashMap<u32, Contribution> {
        let mut out: HashMap<u32, Contribution> = HashMap::new();
        for s in &e.steps {
            let (title, url) = self.template_strings(s.template);
            for b in regions::cut(&self.terms, s.root) {
                let key: Box<[u32]> = b.cids.iter().map(|&c| self.terms.node(c).sid).collect();
                let Some(rid) = self.regions.find(&key).filter(|r| rids.contains(r)) else {
                    continue;
                };
                let entry = out.entry(rid).or_default();
                if entry.1.is_none() {
                    let text = regions::render_batch(&self.terms, &b.cids);
                    entry.1 = Some((b.breadcrumb.clone(), title.clone(), url.clone(), text));
                }
                if !entry.0.iter().any(|v| **v == *b.cids) {
                    entry.0.push(b.cids.as_slice().into());
                }
            }
        }
        out
    }

    /// Rebuilds the variants, values and first-occurrence text of regions from the steps that
    /// still contain them, so nothing of a purged episode survives in a region it shared. The
    /// result is what the store would hold had the episode never been ingested. Each surviving
    /// step is cut once however many regions it feeds.
    fn rederive_regions(&mut self, rids: &BTreeSet<u32>) {
        if rids.is_empty() {
            return;
        }
        let mut steps: BTreeSet<(u32, u32)> = BTreeSet::new();
        for &rid in rids {
            if let Some(r) = self.regions.get(rid) {
                steps.extend(r.postings.iter().map(|p| (p.0, p.1)));
            }
        }
        let mut fresh: BTreeMap<u32, Rederived> = BTreeMap::new();
        for (eid, step) in steps {
            let Some(Some(ep)) = self.episodes.get(eid as usize) else {
                continue;
            };
            let Ok(i) = ep.steps.binary_search_by_key(&step, |s| s.step) else {
                continue;
            };
            let (root, template) = (ep.steps[i].root, ep.steps[i].template);
            let (title, url) = self.template_strings(template);
            for b in regions::cut(&self.terms, root) {
                let key: Box<[u32]> = b.cids.iter().map(|&c| self.terms.node(c).sid).collect();
                let Some(rid) = self.regions.find(&key).filter(|r| rids.contains(r)) else {
                    continue;
                };
                let entry = fresh.entry(rid).or_default();
                if entry.0.is_none() {
                    let text = regions::render_batch(&self.terms, &b.cids);
                    entry.0 = Some((b.breadcrumb.clone(), title.clone(), url.clone(), text));
                }
                if entry.1.len() < regions::MAX_VARIANTS && !entry.1.iter().any(|v| **v == *b.cids)
                {
                    entry.1.push(b.cids.as_slice().into());
                }
            }
        }
        for (rid, (first, variants)) in fresh {
            let Some(first) = first else { continue };
            // Hold the new runs before letting go of the old ones: they share nodes.
            for v in &variants {
                for &c in v.iter() {
                    self.terms.bump(c);
                }
            }
            let mut values: HashMap<String, u32> = HashMap::new();
            for v in &variants {
                let mut vals = Vec::new();
                for &c in v.iter() {
                    regions::value_lines(&self.terms, c, &mut vals);
                }
                for line in vals {
                    *values.entry(trunc(&line, 160).to_string()).or_insert(0) += 1;
                }
            }
            let Some(Some(r)) = self.regions.slots.get_mut(rid as usize) else {
                continue;
            };
            let old = std::mem::replace(&mut r.variants, variants);
            (r.breadcrumb, r.title, r.url, r.text) = first;
            r.values = values;
            let index_text = r.index_text();
            for v in old {
                for &c in v.iter() {
                    self.terms.release(c);
                }
            }
            if let Err(e) = self
                .region_docs
                .memorize(doc(format!("r{rid}"), index_text))
            {
                debug_assert!(false, "region index: {e}");
            }
        }
    }

    /// Removes an episode and everything only it held; shared structure stays. Returns
    /// `None` for an unknown episode.
    pub fn purge_episode(&mut self, id: &str) -> Option<PurgeReport> {
        let eid = self.episode_slot(id)?;
        let nodes_before = self.terms.live_nodes();
        let regions_before = self.regions.live();
        let e = self.episodes[eid as usize].take()?;
        self.step_count -= e.steps.len();
        self.step_region_count -= e.steps.iter().map(|s| s.regions.len()).sum::<usize>();
        for s in &e.steps {
            self.step_controls -= s.controls.len();
            self.step_control_bytes -= s.controls.iter().map(Control::text_bytes).sum::<usize>();
        }
        self.episode_map.remove(id);
        self.free_episodes.push(eid);
        self.dirty.remove(&eid);
        self.episode_docs.delete(&format!("e{eid}"));

        for (i, s) in e.steps.iter().enumerate() {
            if let (Some(l), true) = (s.label, i > 0) {
                let prev = &e.steps[i - 1];
                if let (Some(p), Some(c)) = (self.frames.get(prev.frame), self.frames.get(s.frame))
                {
                    let d = delta(&self.terms.syms, &p.lines, &c.lines);
                    let action = self.terms.syms.get(l).to_string();
                    self.effects.apply(
                        &mut self.terms.syms,
                        &mut self.templates,
                        (prev.template, l, s.template),
                        &d,
                        false,
                    );
                    self.sync_effect_doc(prev.template, &action);
                }
            }
            if let Some(f) = self.frames.get(s.frame) {
                let lines = f.lines.clone();
                self.templates
                    .observe(&mut self.terms.syms, s.template, &lines, false);
            }
        }
        let touched: BTreeSet<u32> = e
            .steps
            .iter()
            .flat_map(|s| s.regions.iter().map(|r| r.0))
            .collect();
        // What this episode contributed to each region it shares: the value variants it saw and
        // the page, breadcrumb and text of its first occurrence.
        let contributed = self.contributions(&e, &touched);
        let mut shared = BTreeSet::new();
        for rid in touched {
            let empty = match self.regions.slots.get_mut(rid as usize) {
                Some(Some(r)) => {
                    r.remove_episode(eid);
                    r.postings.is_empty()
                }
                _ => false,
            };
            if empty {
                self.regions.remove(&mut self.terms, rid);
                self.region_docs.delete(&format!("r{rid}"));
            } else if let (Some(r), Some((variants, first))) =
                (self.regions.get(rid), contributed.get(&rid))
            {
                let held = variants
                    .iter()
                    .any(|v| r.variants.iter().any(|x| **x == **v));
                let source = first.as_ref().is_some_and(|f| {
                    (&f.0, &f.1, &f.2, &f.3) == (&r.breadcrumb, &r.title, &r.url, &r.text)
                });
                if held || source {
                    shared.insert(rid);
                }
            }
        }
        self.rederive_regions(&shared);
        for s in &e.steps {
            self.frames.release(&mut self.terms.syms, s.frame);
            self.templates.release(&mut self.terms.syms, s.template);
            self.terms.release(s.root);
            if let Some(l) = s.label {
                self.terms.syms.release(l);
            }
        }
        Some(PurgeReport {
            steps: e.steps.len(),
            regions_freed: regions_before - self.regions.live(),
            nodes_freed: nodes_before - self.terms.live_nodes(),
        })
    }
}
