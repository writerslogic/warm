// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Page templates with their frame log, interned frames and the effect table.

use std::collections::HashMap;

use super::terms::{Sym, Symbols};
use super::tree::{is_error_line, trunc};

const MAX_LISTED_LINES: usize = 60;
const ERROR_KEY_CHARS: usize = 120;

pub(crate) struct TemplateRec {
    pub url: Sym,
    pub title: Sym,
    pub refs: u32,
    /// Observations of this template: the frame log's count.
    pub obs: u32,
    /// Line to the number of observations that showed it.
    pub inventory: HashMap<Sym, u32>,
}

#[derive(Default)]
pub(crate) struct Templates {
    pub slots: Vec<Option<TemplateRec>>,
    free: Vec<u32>,
    map: HashMap<(Sym, Sym), u32>,
}

fn inc<K: std::hash::Hash + Eq + Copy>(m: &mut HashMap<K, u32>, k: K) -> bool {
    let e = m.entry(k).or_insert(0);
    *e = e.saturating_add(1);
    *e == 1
}

fn dec<K: std::hash::Hash + Eq + Copy>(m: &mut HashMap<K, u32>, k: K) -> bool {
    match m.get_mut(&k) {
        Some(c) if *c > 1 => {
            *c -= 1;
            false
        }
        Some(_) => {
            m.remove(&k);
            true
        }
        None => false,
    }
}

impl Templates {
    pub fn get(&self, id: u32) -> Option<&TemplateRec> {
        self.slots.get(id as usize).and_then(Option::as_ref)
    }

    pub fn live(&self) -> usize {
        self.map.len()
    }

    pub fn lookup(&self, syms: &Symbols, url: &str, title: &str) -> Option<u32> {
        let key = (syms.lookup(url)?, syms.lookup(title)?);
        self.map.get(&key).copied()
    }

    /// Returns the template of the pair with one reference owned by the caller.
    pub fn acquire(&mut self, syms: &mut Symbols, url: &str, title: &str) -> u32 {
        let (u, t) = (syms.acquire(url), syms.acquire(title));
        if let Some(&id) = self.map.get(&(u, t)) {
            syms.release(u);
            syms.release(t);
            self.bump(id);
            return id;
        }
        self.place_new(u, t, 1)
    }

    fn place_new(&mut self, url: Sym, title: Sym, refs: u32) -> u32 {
        let rec = TemplateRec {
            url,
            title,
            refs,
            obs: 0,
            inventory: HashMap::new(),
        };
        let id = match self.free.pop() {
            Some(i) => {
                self.slots[i as usize] = Some(rec);
                i
            }
            None => {
                self.slots.push(Some(rec));
                (self.slots.len() - 1) as u32
            }
        };
        self.map.insert((url, title), id);
        id
    }

    /// Places a template at an exact slot when loading a snapshot; the caller supplies one
    /// reference on each symbol.
    pub fn place(&mut self, id: usize, url: Sym, title: Sym) -> bool {
        while self.slots.len() <= id {
            self.slots.push(None);
        }
        if self.slots[id].is_some() || self.map.contains_key(&(url, title)) {
            return false;
        }
        self.slots[id] = Some(TemplateRec {
            url,
            title,
            refs: 0,
            obs: 0,
            inventory: HashMap::new(),
        });
        self.map.insert((url, title), id as u32);
        true
    }

    pub fn finish_load(&mut self) -> bool {
        self.free = (0..self.slots.len() as u32)
            .rev()
            .filter(|&i| self.slots[i as usize].is_none())
            .collect();
        self.slots.iter().flatten().all(|t| t.refs > 0)
    }

    pub fn bump(&mut self, id: u32) {
        if let Some(Some(t)) = self.slots.get_mut(id as usize) {
            t.refs = t.refs.saturating_add(1);
        }
    }

    pub fn release(&mut self, syms: &mut Symbols, id: u32) {
        let Some(Some(t)) = self.slots.get_mut(id as usize) else {
            return;
        };
        if t.refs == u32::MAX {
            return;
        }
        t.refs -= 1;
        if t.refs == 0 {
            if let Some(t) = self.slots[id as usize].take() {
                self.map.remove(&(t.url, t.title));
                syms.release(t.url);
                syms.release(t.title);
                for (s, _) in t.inventory {
                    syms.release(s);
                }
            }
            self.free.push(id);
        }
    }

    /// Counts one more (`add`) or one fewer observation of `lines` in the template.
    pub fn observe(&mut self, syms: &mut Symbols, id: u32, lines: &[Sym], add: bool) {
        let Some(Some(t)) = self.slots.get_mut(id as usize) else {
            return;
        };
        if add {
            t.obs = t.obs.saturating_add(1);
            for &l in lines {
                if inc(&mut t.inventory, l) {
                    syms.bump(l);
                }
            }
        } else {
            t.obs = t.obs.saturating_sub(1);
            for &l in lines {
                if dec(&mut t.inventory, l) {
                    syms.release(l);
                }
            }
        }
    }
}

pub(crate) struct FrameRec {
    pub lines: Box<[Sym]>,
    pub refs: u32,
}

/// Interned line sets: whole observations repeat, and a repeat costs one reference.
#[derive(Default)]
pub(crate) struct Frames {
    pub slots: Vec<Option<FrameRec>>,
    free: Vec<u32>,
    map: HashMap<Box<[Sym]>, u32>,
}

impl Frames {
    pub fn get(&self, id: u32) -> Option<&FrameRec> {
        self.slots.get(id as usize).and_then(Option::as_ref)
    }

    pub fn live(&self) -> usize {
        self.map.len()
    }

    /// `lines` are sorted ascending, distinct, and each carries one reference owned by the
    /// caller, which this call consumes. Returns the frame with one reference owned by the caller.
    pub fn acquire(&mut self, syms: &mut Symbols, lines: Box<[Sym]>) -> u32 {
        if let Some(&id) = self.map.get(&lines) {
            for &l in lines.iter() {
                syms.release(l);
            }
            if let Some(Some(f)) = self.slots.get_mut(id as usize) {
                f.refs = f.refs.saturating_add(1);
            }
            return id;
        }
        self.place_new(lines, 1)
    }

    fn place_new(&mut self, lines: Box<[Sym]>, refs: u32) -> u32 {
        let rec = FrameRec {
            lines: lines.clone(),
            refs,
        };
        let id = match self.free.pop() {
            Some(i) => {
                self.slots[i as usize] = Some(rec);
                i
            }
            None => {
                self.slots.push(Some(rec));
                (self.slots.len() - 1) as u32
            }
        };
        self.map.insert(lines, id);
        id
    }

    pub fn place(&mut self, id: usize, lines: Box<[Sym]>) -> bool {
        while self.slots.len() <= id {
            self.slots.push(None);
        }
        if self.slots[id].is_some() || self.map.contains_key(&lines) {
            return false;
        }
        self.map.insert(lines.clone(), id as u32);
        self.slots[id] = Some(FrameRec { lines, refs: 0 });
        true
    }

    pub fn bump(&mut self, id: u32) {
        if let Some(Some(f)) = self.slots.get_mut(id as usize) {
            f.refs = f.refs.saturating_add(1);
        }
    }

    pub fn finish_load(&mut self) -> bool {
        self.free = (0..self.slots.len() as u32)
            .rev()
            .filter(|&i| self.slots[i as usize].is_none())
            .collect();
        self.slots.iter().flatten().all(|f| f.refs > 0)
    }

    pub fn release(&mut self, syms: &mut Symbols, id: u32) {
        let Some(Some(f)) = self.slots.get_mut(id as usize) else {
            return;
        };
        if f.refs == u32::MAX {
            return;
        }
        f.refs -= 1;
        if f.refs == 0 {
            if let Some(f) = self.slots[id as usize].take() {
                self.map.remove(&f.lines);
                for &l in f.lines.iter() {
                    syms.release(l);
                }
            }
            self.free.push(id);
        }
    }
}

/// What changed between two consecutive observations of an episode.
pub(crate) struct Delta {
    /// At most 60 added lines, ordered by text.
    pub add: Vec<Sym>,
    pub rem: Vec<Sym>,
    /// Every added line that reads as an error, cut to 120 characters, ordered by text.
    pub errors: Vec<String>,
    pub no_change: bool,
}

pub(crate) fn delta(syms: &Symbols, prev: &[Sym], cur: &[Sym]) -> Delta {
    let (mut add, mut rem) = (Vec::new(), Vec::new());
    let (mut i, mut j) = (0, 0);
    while i < prev.len() || j < cur.len() {
        match (prev.get(i), cur.get(j)) {
            (Some(a), Some(b)) if a == b => {
                i += 1;
                j += 1;
            }
            (Some(a), Some(b)) if a < b => {
                rem.push(*a);
                i += 1;
            }
            (Some(_), Some(b)) => {
                add.push(*b);
                j += 1;
            }
            (Some(a), None) => {
                rem.push(*a);
                i += 1;
            }
            (None, Some(b)) => {
                add.push(*b);
                j += 1;
            }
            (None, None) => break,
        }
    }
    let by_text = |a: &Sym, b: &Sym| syms.get(*a).cmp(syms.get(*b));
    add.sort_by(by_text);
    rem.sort_by(by_text);
    let no_change = add.is_empty() && rem.is_empty();
    let errors = add
        .iter()
        .map(|&l| syms.get(l))
        .filter(|l| is_error_line(l))
        .map(|l| trunc(l, ERROR_KEY_CHARS).to_string())
        .collect();
    add.truncate(MAX_LISTED_LINES);
    rem.truncate(MAX_LISTED_LINES);
    Delta {
        add,
        rem,
        errors,
        no_change,
    }
}

#[derive(Default)]
pub(crate) struct EffectRow {
    pub n: u32,
    pub loops: u32,
    pub add: HashMap<Sym, u32>,
    pub rem: HashMap<Sym, u32>,
    pub nxt: HashMap<u32, u32>,
    pub err: HashMap<Sym, u32>,
}

#[derive(Default)]
pub(crate) struct EffectTable {
    pub rows: HashMap<(u32, Sym), EffectRow>,
}

impl EffectTable {
    /// Adds (`add`) or removes the effect of one transition from template `prev` by action
    /// `label` into template `cur`. The caller holds references on all three for the call.
    pub fn apply(
        &mut self,
        syms: &mut Symbols,
        tmpls: &mut Templates,
        key: (u32, Sym, u32),
        d: &Delta,
        add: bool,
    ) {
        let (prev, label, cur) = key;
        if add {
            if !self.rows.contains_key(&(prev, label)) {
                tmpls.bump(prev);
                syms.bump(label);
            }
            let row = self.rows.entry((prev, label)).or_default();
            row.n = row.n.saturating_add(1);
            row.loops = row.loops.saturating_add(u32::from(d.no_change));
            if inc(&mut row.nxt, cur) {
                tmpls.bump(cur);
            }
            for &l in &d.add {
                if inc(&mut row.add, l) {
                    syms.bump(l);
                }
            }
            for &l in &d.rem {
                if inc(&mut row.rem, l) {
                    syms.bump(l);
                }
            }
            for e in &d.errors {
                let s = syms.acquire(e);
                if !inc(&mut row.err, s) {
                    syms.release(s);
                }
            }
            return;
        }
        let Some(row) = self.rows.get_mut(&(prev, label)) else {
            return;
        };
        row.n = row.n.saturating_sub(1);
        row.loops = row.loops.saturating_sub(u32::from(d.no_change));
        if dec(&mut row.nxt, cur) {
            tmpls.release(syms, cur);
        }
        for &l in &d.add {
            if dec(&mut row.add, l) {
                syms.release(l);
            }
        }
        for &l in &d.rem {
            if dec(&mut row.rem, l) {
                syms.release(l);
            }
        }
        for e in &d.errors {
            let s = syms.acquire(e);
            if dec(&mut row.err, s) {
                syms.release(s);
            }
            syms.release(s);
        }
        if row.n == 0 {
            self.rows.remove(&(prev, label));
            tmpls.release(syms, prev);
            syms.release(label);
        }
    }
}
