// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Cut policy, rendering and the region store.
//!
//! A region is a run of sibling subtrees whose rendered size fits `UNIT_MAX`; its identity is
//! the sequence of their structural ids, so equal fragments from different observations are
//! one region.

use std::collections::HashMap;

use super::terms::{Sym, TermStore};
use super::tree::{char_len, is_skip_role, label, trunc};

pub(crate) const UNIT_MAX: u32 = 1500;
pub(crate) const MAX_VARIANTS: usize = 12;
const MIN_TEXT_CHARS: usize = 3;
const BREADCRUMB_DEPTH: usize = 3;
const BREADCRUMB_CHARS: usize = 200;

pub(crate) struct Batch {
    pub cids: Vec<u32>,
    pub breadcrumb: String,
}

fn s(ts: &TermStore, sym: Sym) -> &str {
    ts.sym_str(sym)
}

/// Cuts the children of `root` into batches along subtree boundaries.
pub(crate) fn cut(ts: &TermStore, root: u32) -> Vec<Batch> {
    fn flush(batch: &mut Vec<u32>, size: &mut u32, path: &[String], out: &mut Vec<Batch>) {
        if !batch.is_empty() {
            let from = path.len().saturating_sub(BREADCRUMB_DEPTH);
            let joined = path[from..].join(" > ");
            out.push(Batch {
                cids: std::mem::take(batch),
                breadcrumb: trunc(&joined, BREADCRUMB_CHARS).to_string(),
            });
        }
        *size = 0;
    }
    fn group(ts: &TermStore, nodes: &[u32], path: &mut Vec<String>, out: &mut Vec<Batch>) {
        let mut batch = Vec::new();
        let mut bs = 0u32;
        for &c in nodes {
            let n = ts.node(c);
            if n.size > UNIT_MAX && !n.children.is_empty() {
                flush(&mut batch, &mut bs, path, out);
                path.push(label(s(ts, n.role), s(ts, n.name)));
                group(ts, &n.children, path, out);
                path.pop();
            } else {
                if bs.saturating_add(n.size) > UNIT_MAX {
                    flush(&mut batch, &mut bs, path, out);
                }
                batch.push(c);
                bs = bs.saturating_add(n.size);
            }
        }
        flush(&mut batch, &mut bs, path, out);
    }
    let r = ts.node(root);
    let mut path = vec![label(s(ts, r.role), s(ts, r.name))];
    let mut out = Vec::new();
    group(ts, &r.children, &mut path, &mut out);
    out
}

pub(crate) fn render(
    ts: &TermStore,
    id: u32,
    depth: usize,
    parent: Option<&str>,
    out: &mut Vec<String>,
) {
    let n = ts.node(id);
    let (role, name) = (s(ts, n.role), s(ts, n.name));
    if role == "StaticText" {
        if !name.trim().is_empty() && Some(name) != parent {
            out.push(format!("{}\"{}\"", " ".repeat(depth), trunc(name, 200)));
        }
        return;
    }
    let show = !name.is_empty() || !is_skip_role(role);
    if show {
        out.push(format!("{}{}", " ".repeat(depth), label(role, name)));
    }
    for &c in n.children.iter() {
        render(ts, c, depth + usize::from(show), Some(name), out);
    }
}

pub(crate) fn render_batch(ts: &TermStore, cids: &[u32]) -> String {
    let mut lines = Vec::new();
    for &c in cids {
        render(ts, c, 0, None, &mut lines);
    }
    lines.join("\n")
}

pub(crate) fn text_is_indexable(text: &str) -> bool {
    char_len(text.trim()) >= MIN_TEXT_CHARS
}

pub(crate) fn value_lines(ts: &TermStore, id: u32, out: &mut Vec<String>) {
    let n = ts.node(id);
    let props = s(ts, n.props);
    if !props.is_empty() {
        out.push(format!("{} {props}", label(s(ts, n.role), s(ts, n.name))));
    }
    for &c in n.children.iter() {
        value_lines(ts, c, out);
    }
}

/// The distinct lines an observation shows: element labels and static text.
pub(crate) fn flat_lines(ts: &TermStore, id: u32, out: &mut std::collections::BTreeSet<String>) {
    let n = ts.node(id);
    let (role, name) = (s(ts, n.role), s(ts, n.name));
    if role == "StaticText" {
        if !name.trim().is_empty() {
            out.insert(format!("\"{}\"", trunc(name, 160)));
        }
    } else if !name.is_empty() {
        out.insert(label(role, name));
    }
    for &c in n.children.iter() {
        flat_lines(ts, c, out);
    }
}

pub(crate) struct Region {
    pub key: Box<[u32]>,
    pub breadcrumb: String,
    pub title: String,
    pub url: String,
    pub text: String,
    pub variants: Vec<Box<[u32]>>,
    pub values: HashMap<String, u32>,
    /// Sorted by (episode, step); the count is how often the region occurs in that step.
    pub postings: Vec<(u32, u32, u32)>,
}

impl Region {
    pub fn occurrences(&self) -> u64 {
        self.postings.iter().map(|p| u64::from(p.2)).sum()
    }

    pub fn episodes(&self) -> usize {
        let mut n = 0;
        let mut last = None;
        for p in &self.postings {
            if last != Some(p.0) {
                n += 1;
                last = Some(p.0);
            }
        }
        n
    }

    /// The text a reader sees: provenance header, the rendered fragment and the value
    /// variants seen.
    pub fn unit_text(&self) -> String {
        let mut x = format!(
            "[page '{}' {}; in {} episodes/{} steps] {}\n{}",
            self.title,
            trunc(&self.url, 60),
            self.episodes(),
            self.postings.len(),
            self.breadcrumb,
            self.text
        );
        if !self.values.is_empty() {
            let mut v: Vec<(&String, &u32)> = self.values.iter().collect();
            v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let top: Vec<&str> = v.iter().take(6).map(|(l, _)| l.as_str()).collect();
            x.push_str("\nvalues seen: ");
            x.push_str(&top.join(" | "));
        }
        x
    }

    pub fn index_text(&self) -> String {
        format!("{} {}\n{}", self.title, self.breadcrumb, self.text)
    }

    pub fn add_posting(&mut self, episode: u32, step: u32, count: u32) {
        let at = self
            .postings
            .partition_point(|p| (p.0, p.1) < (episode, step));
        match self.postings.get_mut(at) {
            Some(p) if (p.0, p.1) == (episode, step) => p.2 = p.2.saturating_add(count),
            _ => self.postings.insert(at, (episode, step, count)),
        }
    }

    pub fn remove_episode(&mut self, episode: u32) {
        let lo = self.postings.partition_point(|p| p.0 < episode);
        let hi = self.postings.partition_point(|p| p.0 <= episode);
        self.postings.drain(lo..hi);
    }
}

#[derive(Default)]
pub(crate) struct RegionStore {
    pub slots: Vec<Option<Region>>,
    free: Vec<u32>,
    map: HashMap<Box<[u32]>, u32>,
}

impl RegionStore {
    pub fn find(&self, key: &[u32]) -> Option<u32> {
        self.map.get(key).copied()
    }

    pub fn get(&self, id: u32) -> Option<&Region> {
        self.slots.get(id as usize).and_then(Option::as_ref)
    }

    pub fn live(&self) -> usize {
        self.map.len()
    }

    /// Adds a region whose structural key references are already held by the caller.
    pub fn insert(&mut self, region: Region) -> u32 {
        let key = region.key.clone();
        let id = match self.free.pop() {
            Some(i) => {
                self.slots[i as usize] = Some(region);
                i
            }
            None => {
                self.slots.push(Some(region));
                (self.slots.len() - 1) as u32
            }
        };
        self.map.insert(key, id);
        id
    }

    /// Places a region at an exact slot when loading a snapshot; false if the slot or key is taken.
    pub fn place(&mut self, id: usize, region: Region) -> bool {
        while self.slots.len() <= id {
            self.slots.push(None);
        }
        if self.slots[id].is_some() || self.map.contains_key(&region.key) {
            return false;
        }
        self.map.insert(region.key.clone(), id as u32);
        self.slots[id] = Some(region);
        true
    }

    pub fn finish_load(&mut self) {
        self.free = (0..self.slots.len() as u32)
            .rev()
            .filter(|&i| self.slots[i as usize].is_none())
            .collect();
    }

    /// Removes a region and releases what it holds in `ts`.
    pub fn remove(&mut self, ts: &mut TermStore, id: u32) {
        let Some(r) = self.slots.get_mut(id as usize).and_then(Option::take) else {
            return;
        };
        self.map.remove(&r.key);
        self.free.push(id);
        for &sid in r.key.iter() {
            ts.release_sid(sid);
        }
        for v in &r.variants {
            for &c in v.iter() {
                ts.release(c);
            }
        }
    }
}
