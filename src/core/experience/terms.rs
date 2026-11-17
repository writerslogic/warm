// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Reference-counted symbol table and hash-consed term arena.
//!
//! A reference count saturates at `u32::MAX` and a saturated count never drops again, so a
//! count that cannot be represented leaks the node instead of freeing a live one.

use std::collections::HashMap;
use std::sync::Arc;

use super::tree::{own_size, TreeNode};

pub(crate) type Sym = u32;

#[derive(Default)]
pub(crate) struct Symbols {
    strings: Vec<Option<Arc<str>>>,
    refs: Vec<u32>,
    map: HashMap<Arc<str>, Sym>,
    free: Vec<Sym>,
    /// Bytes of the live strings.
    bytes: usize,
}

fn up(r: &mut u32) {
    *r = r.saturating_add(1);
}

impl Symbols {
    /// Returns the symbol of `s` with one more reference owned by the caller.
    pub fn acquire(&mut self, s: &str) -> Sym {
        if let Some(&id) = self.map.get(s) {
            up(&mut self.refs[id as usize]);
            return id;
        }
        let a: Arc<str> = Arc::from(s);
        let id = match self.free.pop() {
            Some(i) => {
                self.strings[i as usize] = Some(a.clone());
                self.refs[i as usize] = 1;
                i
            }
            None => {
                self.strings.push(Some(a.clone()));
                self.refs.push(1);
                (self.strings.len() - 1) as Sym
            }
        };
        self.map.insert(a, id);
        self.bytes += s.len();
        id
    }

    pub fn bump(&mut self, id: Sym) {
        up(&mut self.refs[id as usize]);
    }

    pub fn release(&mut self, id: Sym) {
        let r = &mut self.refs[id as usize];
        if *r == u32::MAX {
            return;
        }
        *r -= 1;
        if *r == 0 {
            if let Some(s) = self.strings[id as usize].take() {
                self.map.remove(&*s);
                self.bytes -= s.len();
            }
            self.free.push(id);
        }
    }

    pub fn get(&self, id: Sym) -> &str {
        self.strings[id as usize].as_deref().unwrap_or("")
    }

    pub fn lookup(&self, s: &str) -> Option<Sym> {
        self.map.get(s).copied()
    }

    pub fn live(&self) -> usize {
        self.map.len()
    }

    pub fn live_bytes(&self) -> usize {
        self.bytes
    }

    pub fn slots(&self) -> impl Iterator<Item = Option<&str>> {
        self.strings.iter().map(|s| s.as_deref())
    }

    /// Places `s` at slot `id` with no references; used when loading a snapshot.
    pub fn place(&mut self, id: usize, s: Option<&str>) -> bool {
        while self.strings.len() <= id {
            self.strings.push(None);
            self.refs.push(0);
        }
        if let Some(s) = s {
            if self.map.contains_key(s) {
                return false;
            }
            let a: Arc<str> = Arc::from(s);
            self.strings[id] = Some(a.clone());
            self.map.insert(a, id as Sym);
            self.bytes += s.len();
        }
        true
    }

    pub fn finish_load(&mut self) -> bool {
        self.free = (0..self.strings.len() as Sym)
            .rev()
            .filter(|&i| self.strings[i as usize].is_none())
            .collect();
        self.strings
            .iter()
            .zip(&self.refs)
            .all(|(s, &r)| s.is_none() || r > 0)
    }
}

pub(crate) struct NodeRec {
    pub role: Sym,
    pub name: Sym,
    pub props: Sym,
    pub children: Box<[u32]>,
    pub sid: u32,
    pub size: u32,
    pub refs: u32,
}

struct StructRec {
    key: (Sym, Sym, Box<[u32]>),
    refs: u32,
}

type ContentKey = (Sym, Sym, Sym, Box<[u32]>);

#[derive(Default)]
pub(crate) struct TermStore {
    pub syms: Symbols,
    nodes: Vec<Option<NodeRec>>,
    free_nodes: Vec<u32>,
    content: HashMap<ContentKey, u32>,
    structs: Vec<Option<StructRec>>,
    free_structs: Vec<u32>,
    struct_map: HashMap<(Sym, Sym, Box<[u32]>), u32>,
}

impl TermStore {
    pub fn live_nodes(&self) -> usize {
        self.content.len()
    }

    pub fn live_structs(&self) -> usize {
        self.struct_map.len()
    }

    pub fn node(&self, id: u32) -> &NodeRec {
        self.nodes[id as usize].as_ref().expect("live node id")
    }

    pub fn try_node(&self, id: u32) -> Option<&NodeRec> {
        self.nodes.get(id as usize).and_then(Option::as_ref)
    }

    pub fn slots(&self) -> impl Iterator<Item = Option<&NodeRec>> {
        self.nodes.iter().map(Option::as_ref)
    }

    pub fn bump(&mut self, id: u32) {
        if let Some(Some(n)) = self.nodes.get_mut(id as usize) {
            up(&mut n.refs);
        }
    }

    fn struct_id(&mut self, role: Sym, name: Sym, children: &[u32]) -> u32 {
        let kids: Box<[u32]> = children.iter().map(|&c| self.node(c).sid).collect();
        let key = (role, name, kids);
        if let Some(&sid) = self.struct_map.get(&key) {
            if let Some(Some(r)) = self.structs.get_mut(sid as usize) {
                up(&mut r.refs);
            }
            return sid;
        }
        let rec = StructRec {
            key: key.clone(),
            refs: 1,
        };
        let sid = match self.free_structs.pop() {
            Some(i) => {
                self.structs[i as usize] = Some(rec);
                i
            }
            None => {
                self.structs.push(Some(rec));
                (self.structs.len() - 1) as u32
            }
        };
        self.struct_map.insert(key, sid);
        sid
    }

    pub fn bump_sid(&mut self, sid: u32) {
        if let Some(Some(r)) = self.structs.get_mut(sid as usize) {
            up(&mut r.refs);
        }
    }

    pub fn release_sid(&mut self, sid: u32) {
        let Some(Some(r)) = self.structs.get_mut(sid as usize) else {
            return;
        };
        if r.refs == u32::MAX {
            return;
        }
        r.refs -= 1;
        if r.refs == 0 {
            if let Some(rec) = self.structs[sid as usize].take() {
                self.struct_map.remove(&rec.key);
            }
            self.free_structs.push(sid);
        }
    }

    /// Interns a tree and returns the root content id with one reference owned by the caller.
    /// The tree must have passed the depth limit, which bounds the recursion.
    pub fn intern(&mut self, t: &TreeNode) -> u32 {
        let kids: Vec<u32> = t.children.iter().map(|c| self.intern(c)).collect();
        let role = self.syms.acquire(&t.role);
        let name = self.syms.acquire(&t.name);
        let props = self.syms.acquire(&t.props);
        self.intern_parts(role, name, props, kids)
    }

    /// `role`, `name`, `props` and every child carry one reference owned by the caller, which
    /// this call consumes.
    pub fn intern_parts(&mut self, role: Sym, name: Sym, props: Sym, kids: Vec<u32>) -> u32 {
        let key: ContentKey = (role, name, props, kids.into_boxed_slice());
        if let Some(&id) = self.content.get(&key) {
            up(&mut self.nodes[id as usize].as_mut().expect("indexed node").refs);
            for s in [role, name, props] {
                self.syms.release(s);
            }
            for &c in key.3.iter() {
                self.release(c);
            }
            return id;
        }
        let sid = self.struct_id(role, name, &key.3);
        let own = own_size(self.syms.get(role), self.syms.get(name));
        let size = key
            .3
            .iter()
            .fold(own, |a, &c| a.saturating_add(self.node(c).size));
        let rec = NodeRec {
            role,
            name,
            props,
            children: key.3.clone(),
            sid,
            size,
            refs: 1,
        };
        let id = match self.free_nodes.pop() {
            Some(i) => {
                self.nodes[i as usize] = Some(rec);
                i
            }
            None => {
                self.nodes.push(Some(rec));
                (self.nodes.len() - 1) as u32
            }
        };
        self.content.insert(key, id);
        id
    }

    pub fn release(&mut self, id: u32) {
        let mut stack = vec![id];
        while let Some(id) = stack.pop() {
            let Some(Some(n)) = self.nodes.get_mut(id as usize) else {
                continue;
            };
            if n.refs == u32::MAX {
                continue;
            }
            n.refs -= 1;
            if n.refs > 0 {
                continue;
            }
            let Some(n) = self.nodes[id as usize].take() else {
                continue;
            };
            self.content
                .remove(&(n.role, n.name, n.props, n.children.clone()));
            self.free_nodes.push(id);
            self.release_sid(n.sid);
            for s in [n.role, n.name, n.props] {
                self.syms.release(s);
            }
            stack.extend(n.children.iter().copied());
        }
    }

    /// Slot-exact placement for snapshot loading: the node starts with no external
    /// references, and its children gain one reference each.
    pub fn place_node(&mut self, id: u32, role: Sym, name: Sym, props: Sym, kids: &[u32]) -> bool {
        let id = id as usize;
        while self.nodes.len() <= id {
            self.nodes.push(None);
        }
        if self.nodes[id].is_some() {
            return false;
        }
        for s in [role, name, props] {
            self.syms.bump(s);
        }
        for &c in kids {
            self.bump(c);
        }
        let key: ContentKey = (role, name, props, kids.into());
        if self.content.contains_key(&key) {
            return false;
        }
        let sid = self.struct_id(role, name, kids);
        let own = own_size(self.syms.get(role), self.syms.get(name));
        let size = kids
            .iter()
            .fold(own, |a, &c| a.saturating_add(self.node(c).size));
        self.nodes[id] = Some(NodeRec {
            role,
            name,
            props,
            children: kids.into(),
            sid,
            size,
            refs: 0,
        });
        self.content.insert(key, id as u32);
        true
    }

    pub fn finish_load(&mut self) -> bool {
        self.free_nodes = (0..self.nodes.len() as u32)
            .rev()
            .filter(|&i| self.nodes[i as usize].is_none())
            .collect();
        self.nodes.iter().flatten().all(|n| n.refs > 0)
    }

    pub fn sym_str(&self, s: Sym) -> &str {
        self.syms.get(s)
    }
}
