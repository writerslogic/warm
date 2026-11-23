// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Row-aware differences between two observations of one episode.
//!
//! The listed lines of an observation lose which row a value sits in and every property such as
//! `checked`, so a swap of assignees between rows or a toggled checkbox changes nothing there.
//! This walks the interned trees instead. A node is addressed by the chain of (role, name,
//! ordinal among siblings with the same role and name). An address is `exact` when no node on the
//! chain shares its role and name with a sibling in either observation; otherwise the match is
//! positional and only consistent with the evidence, not confirmed as the same record.

use super::Experience;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Props { before: String, after: String },
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// `role 'name'#ordinal` of each node from the root, joined by ` > `.
    pub path: String,
    pub kind: ChangeKind,
    /// False when a node on the path had a same-labelled sibling, so the match is positional.
    pub exact: bool,
}

struct Slot {
    id: u32,
    exact: bool,
}

impl Experience {
    fn children_by_address(&self, id: u32, prefix: &str, exact: bool) -> BTreeMap<String, Slot> {
        let mut ord: BTreeMap<(String, String), usize> = BTreeMap::new();
        let mut total: BTreeMap<(String, String), usize> = BTreeMap::new();
        let kids = &self.terms.node(id).children;
        let key = |c: u32| {
            let n = self.terms.node(c);
            (
                self.terms.sym_str(n.role).to_string(),
                self.terms.sym_str(n.name).to_string(),
            )
        };
        for &c in kids.iter() {
            *total.entry(key(c)).or_default() += 1;
        }
        let mut out = BTreeMap::new();
        for &c in kids.iter() {
            let k = key(c);
            let o = ord.entry(k.clone()).or_default();
            let addr = format!("{prefix}{} '{}'#{o}", k.0, k.1);
            let unique = total[&k] == 1;
            *o += 1;
            out.insert(
                addr,
                Slot {
                    id: c,
                    exact: exact && unique,
                },
            );
        }
        out
    }

    fn diff_nodes(&self, a: u32, b: u32, path: &str, exact: bool, out: &mut Vec<Change>) {
        if a == b {
            return;
        }
        let (na, nb) = (self.terms.node(a), self.terms.node(b));
        let (pa, pb) = (self.terms.sym_str(na.props), self.terms.sym_str(nb.props));
        if pa != pb {
            out.push(Change {
                path: path.to_string(),
                kind: ChangeKind::Props {
                    before: pa.to_string(),
                    after: pb.to_string(),
                },
                exact,
            });
        }
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{path} > ")
        };
        let left = self.children_by_address(a, &prefix, exact);
        let right = self.children_by_address(b, &prefix, exact);
        for (addr, l) in &left {
            match right.get(addr) {
                Some(r) => self.diff_nodes(l.id, r.id, addr, l.exact && r.exact, out),
                None => out.push(Change {
                    path: addr.clone(),
                    kind: ChangeKind::Removed,
                    exact: l.exact,
                }),
            }
        }
        for (addr, r) in &right {
            if !left.contains_key(addr) {
                out.push(Change {
                    path: addr.clone(),
                    kind: ChangeKind::Added,
                    exact: r.exact,
                });
            }
        }
    }

    /// What changed between two steps of an episode, by address. `None` when either step is not
    /// stored. Identical content short-circuits through the interned root.
    pub fn state_diff(&self, episode: &str, from: u32, to: u32) -> Option<Vec<Change>> {
        let e = self.ep(self.episode_slot(episode)?);
        let root = |step: u32| {
            e.steps
                .binary_search_by_key(&step, |s| s.step)
                .ok()
                .map(|i| e.steps[i].root)
        };
        let (a, b) = (root(from)?, root(to)?);
        let mut out = Vec::new();
        self.diff_nodes(a, b, "", true, &mut out);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tree::TreeNode;
    use super::super::{Action, Experience, Observation};
    use super::*;

    fn row(id: &str, who: &str) -> TreeNode {
        TreeNode::new("row", id).with_children(vec![TreeNode::new("StaticText", who)])
    }

    fn table(rows: Vec<TreeNode>) -> TreeNode {
        TreeNode::new("RootWebArea", "Tickets")
            .with_children(vec![TreeNode::new("table", "Tickets").with_children(rows)])
    }

    fn put(x: &mut Experience, step: u32, t: &TreeNode) {
        let act = Action::Label("button 'Go'".into());
        x.ingest(&Observation {
            episode: "e",
            step,
            tree: t,
            url: Some("http://h/t"),
            action: (step > 0).then_some(&act),
        })
        .unwrap();
    }

    #[test]
    fn a_swap_of_assignees_between_rows_is_a_change_the_line_set_cannot_see() {
        let mut x = Experience::default();
        put(
            &mut x,
            0,
            &table(vec![row("INC1", "Alice"), row("INC2", "Bob")]),
        );
        put(
            &mut x,
            1,
            &table(vec![row("INC1", "Bob"), row("INC2", "Alice")]),
        );
        let path = x.episode("e").unwrap().path;
        assert!(path[1].no_change && path[1].state_changed);
        let d = x.state_diff("e", 0, 1).unwrap();
        assert_eq!(d.len(), 4, "{d:?}");
        assert!(d.iter().all(|c| c.exact));
        assert!(d.iter().any(|c| c.kind == ChangeKind::Added
            && c.path.contains("row 'INC1'#0")
            && c.path.ends_with("StaticText 'Bob'#0")));
        assert!(d.iter().any(|c| c.kind == ChangeKind::Removed
            && c.path.contains("row 'INC2'#0")
            && c.path.ends_with("StaticText 'Bob'#0")));
    }

    #[test]
    fn same_labelled_siblings_are_matched_by_position_and_say_so() {
        let mut x = Experience::default();
        let r = |who: &str| {
            TreeNode::new("row", "").with_children(vec![TreeNode::new("StaticText", who)])
        };
        put(&mut x, 0, &table(vec![r("Alice"), r("Bob")]));
        put(&mut x, 1, &table(vec![r("Bob"), r("Alice")]));
        let d = x.state_diff("e", 0, 1).unwrap();
        assert!(!d.is_empty() && d.iter().all(|c| !c.exact), "{d:?}");
    }

    #[test]
    fn a_checkbox_toggle_is_a_property_change_and_identical_content_is_empty() {
        let mut x = Experience::default();
        let off = TreeNode::new("RootWebArea", "P")
            .with_children(vec![TreeNode::new("checkbox", "Agree")]);
        let mut on = off.clone();
        on.children[0].props = "checked".into();
        put(&mut x, 0, &off);
        put(&mut x, 1, &on);
        put(&mut x, 2, &on);
        let d = x.state_diff("e", 0, 1).unwrap();
        assert_eq!(
            d[0].kind,
            ChangeKind::Props {
                before: String::new(),
                after: "checked".into()
            }
        );
        assert!(x.state_diff("e", 1, 2).unwrap().is_empty());
        assert!(x.state_diff("e", 0, 9).is_none());
    }
}
