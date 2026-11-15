// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! The persisted form: slot-exact arenas, so identifiers survive a reload, plus the
//! per-episode steps. Reference counts, postings, the effect table, the frame log and the
//! lexical indexes are derived and rebuilt by replaying the steps on load.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::effects::delta;
use super::regions::{self, Region};
use super::{
    doc, index_err, Episode, Experience, ExperienceConfig, ExperienceError, Result, StepRec,
    FLAG_ERROR, FLAG_NO_CHANGE,
};
use crate::core::documents::MemoryDocumentStore;

pub const SNAPSHOT_FILE: &str = "experience.wexp";
const MAGIC: &[u8; 4] = b"WEXP";
const VERSION: u8 = 1;
const HEADER: usize = 4 + 1 + 3 + 8 + 4;
const MAX_SNAPSHOT_BYTES: u64 = 1 << 30;

#[derive(Serialize, Deserialize)]
pub(super) struct SnapNode(pub u32, pub u32, pub u32, pub Vec<u32>);

#[derive(Serialize, Deserialize)]
struct SnapRegion {
    breadcrumb: String,
    title: String,
    url: String,
    text: String,
    variants: Vec<Vec<u32>>,
}

#[derive(Serialize, Deserialize)]
struct SnapStep {
    step: u32,
    root: u32,
    template: u32,
    frame: u32,
    label: Option<u32>,
    regions: Vec<(u32, u32)>,
}

#[derive(Serialize, Deserialize)]
struct SnapEpisode {
    key: String,
    goal: String,
    outcome: String,
    steps: Vec<SnapStep>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Snap {
    symbols: Vec<Option<String>>,
    pub nodes: Vec<Option<SnapNode>>,
    templates: Vec<Option<(u32, u32)>>,
    frames: Vec<Option<Vec<u32>>>,
    regions: Vec<Option<SnapRegion>>,
    episodes: Vec<Option<SnapEpisode>>,
}

fn bad(msg: &str) -> ExperienceError {
    ExperienceError::Snapshot(msg.to_string())
}

impl Experience {
    /// The versioned, checksummed snapshot: a 20-byte header (magic, version, payload length,
    /// CRC32 of the payload) and a CBOR payload.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let snap = Snap {
            symbols: self
                .terms
                .syms
                .slots()
                .map(|s| s.map(str::to_string))
                .collect(),
            nodes: self
                .terms
                .slots()
                .map(|n| n.map(|n| SnapNode(n.role, n.name, n.props, n.children.to_vec())))
                .collect(),
            templates: self
                .templates
                .slots
                .iter()
                .map(|t| t.as_ref().map(|t| (t.url, t.title)))
                .collect(),
            frames: self
                .frames
                .slots
                .iter()
                .map(|f| f.as_ref().map(|f| f.lines.to_vec()))
                .collect(),
            regions: self
                .regions
                .slots
                .iter()
                .map(|r| {
                    r.as_ref().map(|r| SnapRegion {
                        breadcrumb: r.breadcrumb.clone(),
                        title: r.title.clone(),
                        url: r.url.clone(),
                        text: r.text.clone(),
                        variants: r.variants.iter().map(|v| v.to_vec()).collect(),
                    })
                })
                .collect(),
            episodes: self
                .episodes
                .iter()
                .map(|e| {
                    e.as_ref().map(|e| SnapEpisode {
                        key: e.key.clone(),
                        goal: e.goal.clone(),
                        outcome: e.outcome.clone(),
                        steps: e
                            .steps
                            .iter()
                            .map(|s| SnapStep {
                                step: s.step,
                                root: s.root,
                                template: s.template,
                                frame: s.frame,
                                label: s.label,
                                regions: s.regions.clone(),
                            })
                            .collect(),
                    })
                })
                .collect(),
        };
        let mut payload = Vec::new();
        ciborium::into_writer(&snap, &mut payload).map_err(|e| bad(&e.to_string()))?;
        let mut out = Vec::with_capacity(HEADER + payload.len());
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.extend_from_slice(&[0; 3]);
        out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        out.extend_from_slice(&crc32fast::hash(&payload).to_le_bytes());
        out.extend_from_slice(&payload);
        Ok(out)
    }

    /// Writes the snapshot into `dir` atomically: a crash leaves the previous snapshot or the
    /// new one, never a mix.
    pub fn save(&self, dir: &Path) -> Result<()> {
        let bytes = self.to_bytes()?;
        std::fs::create_dir_all(dir).map_err(|e| ExperienceError::Io(e.to_string()))?;
        crate::core::durable_file::atomic_write(&dir.join(SNAPSHOT_FILE), &bytes)
            .map_err(|e| ExperienceError::Io(e.to_string()))
    }

    /// Opens the snapshot in `dir`, or an empty store when there is none. A damaged
    /// snapshot is an error, never an empty store.
    pub fn open(dir: &Path, cfg: ExperienceConfig) -> Result<Self> {
        let path = dir.join(SNAPSHOT_FILE);
        match std::fs::metadata(&path) {
            Ok(m) if m.len() > MAX_SNAPSHOT_BYTES => Err(bad("snapshot exceeds the size limit")),
            Ok(_) => Self::from_bytes(
                &std::fs::read(&path).map_err(|e| ExperienceError::Io(e.to_string()))?,
                cfg,
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new(cfg)),
            Err(e) => Err(ExperienceError::Io(e.to_string())),
        }
    }

    pub fn from_bytes(bytes: &[u8], cfg: ExperienceConfig) -> Result<Self> {
        if bytes.len() < HEADER || &bytes[..4] != MAGIC {
            return Err(bad("not an experience snapshot"));
        }
        if bytes[4] != VERSION {
            return Err(bad("unsupported snapshot version"));
        }
        let len = u64::from_le_bytes(bytes[8..16].try_into().expect("8 bytes"));
        let crc = u32::from_le_bytes(bytes[16..20].try_into().expect("4 bytes"));
        let payload = &bytes[HEADER..];
        if len != payload.len() as u64 || len > MAX_SNAPSHOT_BYTES {
            return Err(bad("payload length does not match the header"));
        }
        if crc32fast::hash(payload) != crc {
            return Err(bad("checksum mismatch"));
        }
        let snap: Snap = ciborium::from_reader(payload).map_err(|e| bad(&e.to_string()))?;
        Self::rebuild(snap, cfg)
    }

    pub(super) fn rebuild(snap: Snap, cfg: ExperienceConfig) -> Result<Self> {
        let mut x = Self::new(cfg);
        let live = |v: &[Option<String>], i: u32| v.get(i as usize).is_some_and(Option::is_some);
        for (i, s) in snap.symbols.iter().enumerate() {
            if !x.terms.syms.place(i, s.as_deref()) {
                return Err(bad("duplicate symbol"));
            }
        }
        for n in snap.nodes.iter().flatten() {
            if ![n.0, n.1, n.2].iter().all(|&s| live(&snap.symbols, s)) {
                return Err(bad("node references a missing symbol"));
            }
            if n.3
                .iter()
                .any(|&c| !snap.nodes.get(c as usize).is_some_and(Option::is_some))
            {
                return Err(bad("node references a missing child"));
            }
        }
        // Children first: an iterative post-order walk that also detects cycles.
        let mut state = vec![0u8; snap.nodes.len()];
        let mut depth = vec![0usize; snap.nodes.len()];
        for root in 0..snap.nodes.len() {
            if snap.nodes[root].is_none() || state[root] == 2 {
                continue;
            }
            let mut stack = vec![(root, 0usize)];
            state[root] = 1;
            while let Some((id, next)) = stack.pop() {
                let n = snap.nodes[id].as_ref().expect("checked live");
                if let Some(&c) = n.3.get(next) {
                    stack.push((id, next + 1));
                    let c = c as usize;
                    match state[c] {
                        1 => return Err(bad("node cycle")),
                        0 => {
                            state[c] = 1;
                            stack.push((c, 0));
                        }
                        _ => {}
                    }
                } else {
                    depth[id] = 1 + n.3.iter().map(|&c| depth[c as usize]).max().unwrap_or(0);
                    if depth[id] > x.cfg.max_depth {
                        return Err(bad("tree deeper than the depth limit"));
                    }
                    state[id] = 2;
                    if !x.terms.place_node(id as u32, n.0, n.1, n.2, &n.3) {
                        return Err(bad("duplicate node"));
                    }
                }
            }
        }
        for (i, t) in snap.templates.iter().enumerate() {
            if let Some(&(u, t)) = t.as_ref() {
                if !live(&snap.symbols, u) || !live(&snap.symbols, t) {
                    return Err(bad("template references a missing symbol"));
                }
                x.terms.syms.bump(u);
                x.terms.syms.bump(t);
                if !x.templates.place(i, u, t) {
                    return Err(bad("duplicate template"));
                }
            }
        }
        for (i, f) in snap.frames.iter().enumerate() {
            if let Some(lines) = f {
                if !lines.windows(2).all(|w| w[0] < w[1])
                    || !lines.iter().all(|&l| live(&snap.symbols, l))
                {
                    return Err(bad("invalid frame"));
                }
                for &l in lines {
                    x.terms.syms.bump(l);
                }
                if !x.frames.place(i, lines.clone().into_boxed_slice()) {
                    return Err(bad("duplicate frame"));
                }
            }
        }
        for (i, r) in snap.regions.iter().enumerate() {
            let Some(r) = r else { continue };
            if r.variants.is_empty()
                || r.variants.len() > regions::MAX_VARIANTS
                || r.variants.iter().any(|v| {
                    v.len() != r.variants[0].len()
                        || v.iter().any(|&c| x.terms.try_node(c).is_none())
                })
            {
                return Err(bad("invalid region variants"));
            }
            let key: Box<[u32]> = r.variants[0].iter().map(|&c| x.terms.node(c).sid).collect();
            for &sid in key.iter() {
                x.terms.bump_sid(sid);
            }
            let region = Region {
                key,
                breadcrumb: r.breadcrumb.clone(),
                title: r.title.clone(),
                url: r.url.clone(),
                text: r.text.clone(),
                variants: Vec::new(),
                values: HashMap::new(),
                postings: Vec::new(),
            };
            if !x.regions.place(i, region) {
                return Err(bad("duplicate region"));
            }
            for v in &r.variants {
                x.observe_variant(i as u32, v);
            }
        }
        for (i, e) in snap.episodes.iter().enumerate() {
            let Some(e) = e else { continue };
            if x.episode_map.contains_key(&e.key) || e.key.is_empty() {
                return Err(bad("duplicate or empty episode id"));
            }
            let mut steps = Vec::with_capacity(e.steps.len());
            for s in &e.steps {
                let ok = x.terms.try_node(s.root).is_some()
                    && x.templates.get(s.template).is_some()
                    && x.frames.get(s.frame).is_some()
                    && s.label.is_none_or(|l| live(&snap.symbols, l))
                    && s.regions.iter().all(|r| x.regions.get(r.0).is_some())
                    && steps.last().is_none_or(|p: &StepRec| p.step < s.step);
                if !ok {
                    return Err(bad("invalid step"));
                }
                x.terms.bump(s.root);
                x.templates.bump(s.template);
                x.frames.bump(s.frame);
                if let Some(l) = s.label {
                    x.terms.syms.bump(l);
                }
                steps.push(StepRec {
                    step: s.step,
                    root: s.root,
                    template: s.template,
                    frame: s.frame,
                    label: s.label,
                    flags: 0,
                    regions: s.regions.clone(),
                });
            }
            while x.episodes.len() <= i {
                x.episodes.push(None);
            }
            x.episodes[i] = Some(Episode {
                key: e.key.clone(),
                goal: e.goal.clone(),
                outcome: e.outcome.clone(),
                steps,
                handles: HashMap::new(),
            });
            x.episode_map.insert(e.key.clone(), i as u32);
            x.dirty.insert(i as u32);
        }
        x.free_episodes = (0..x.episodes.len() as u32)
            .rev()
            .filter(|&i| x.episodes[i as usize].is_none())
            .collect();
        x.regions.finish_load();
        if !(x.terms.syms.finish_load()
            && x.terms.finish_load()
            && x.templates.finish_load()
            && x.frames.finish_load())
        {
            return Err(bad("unreferenced entry"));
        }
        x.replay()?;
        Ok(x)
    }

    /// Recomputes everything derived from the steps.
    fn replay(&mut self) -> Result<()> {
        for eid in 0..self.episodes.len() as u32 {
            let Some(e) = self.episodes[eid as usize].take() else {
                continue;
            };
            let mut steps = e.steps;
            for i in 0..steps.len() {
                let s = &steps[i];
                let lines = self.frames.get(s.frame).expect("validated").lines.clone();
                self.templates
                    .observe(&mut self.terms.syms, s.template, &lines, true);
                let mut flags = 0;
                if let (Some(l), true) = (s.label, i > 0) {
                    let prev = &steps[i - 1];
                    let prev_lines = &self.frames.get(prev.frame).expect("validated").lines;
                    let d = delta(&self.terms.syms, prev_lines, &lines);
                    if d.no_change {
                        flags |= FLAG_NO_CHANGE;
                    }
                    if !d.errors.is_empty() {
                        flags |= FLAG_ERROR;
                    }
                    self.effects.apply(
                        &mut self.terms.syms,
                        &mut self.templates,
                        (prev.template, l, s.template),
                        &d,
                        true,
                    );
                }
                for &(rid, n) in &s.regions {
                    if let Some(Some(r)) = self.regions.slots.get_mut(rid as usize) {
                        r.add_posting(eid, s.step, n);
                    }
                }
                steps[i].flags = flags;
            }
            self.episodes[eid as usize] = Some(Episode { steps, ..e });
        }
        if self
            .regions
            .slots
            .iter()
            .flatten()
            .any(|r| r.postings.is_empty())
        {
            return Err(bad("region without an occurrence"));
        }
        self.region_docs = {
            let mut d =
                MemoryDocumentStore::new(super::INDEX_DIMENSIONS, None).map_err(index_err)?;
            for (i, r) in self.regions.slots.iter().enumerate() {
                if let Some(r) = r {
                    d.memorize(doc(format!("r{i}"), r.index_text()))
                        .map_err(index_err)?;
                }
            }
            d
        };
        Ok(())
    }
}
