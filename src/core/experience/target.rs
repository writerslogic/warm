// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! The authoritative record of an action and the occurrence it targeted.
//!
//! An action stored with an observation acts on the previous observation of the episode. Its
//! target is addressed as that observation's step plus the ordered child indices from its root,
//! an occurrence in an immutable tree. A content id, a structural id, a `role 'name'` label or a
//! bare interaction handle cannot do this: identical sibling subtrees share the first two, same
//! named controls share the third, and handles are reused from one observation to the next.
//! Labels stay display text derived from the action. An occurrence is not an entity: two
//! occurrences are never asserted to describe the same record here.

use super::tree::{label, TreeNode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetRole {
    Subject,
    Destination,
}

/// Why a target has no exact address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unresolved {
    /// The action is the first stored step of its episode.
    NoPredecessor,
    /// The previous stored step is not the step before this one, so the capture has a gap.
    Discontinuity,
    /// The previous observation has no element with this handle.
    HandleNotFound,
    /// Two elements of the previous observation carry this handle.
    HandleConflict,
    /// The previous observation came from a store that did not record element addresses.
    NoHandleContext,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    /// The child indices from the predecessor's root to the targeted node.
    Resolved {
        path: Vec<u32>,
    },
    Unresolved(Unresolved),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionTarget {
    pub role: TargetRole,
    /// The handle the action named, as given.
    pub handle: String,
    pub resolution: Resolution,
}

/// What a step recorded about the action that produced it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionRecord {
    /// The step has no action.
    None,
    /// A step restored from a snapshot that kept only the label: the target is unknown.
    Legacy,
    /// A caller-supplied label without target metadata: nothing to resolve.
    Label(String),
    Call {
        call: String,
        /// The step the action acted on, when the capture is continuous.
        predecessor: Option<u32>,
        targets: Vec<ActionTarget>,
    },
}

impl ActionRecord {
    /// Whether this record came from `action`, comparing the original input and not the label.
    pub(crate) fn is_input(&self, action: Option<&super::Action>) -> Option<bool> {
        use super::Action;
        match (self, action) {
            (Self::None, None) => Some(true),
            (Self::Legacy, _) => None,
            (Self::Label(a), Some(Action::Label(b))) => Some(a == b),
            (Self::Call { call, .. }, Some(Action::Call(b))) => Some(call == b),
            _ => Some(false),
        }
    }

    /// An over-estimate of the encoded size.
    pub(crate) fn encoded_bound(&self) -> usize {
        match self {
            Self::None | Self::Legacy => 8,
            Self::Label(l) => l.len() + 16,
            Self::Call { call, targets, .. } => {
                call.len()
                    + 32
                    + targets
                        .iter()
                        .map(|t| {
                            t.handle.len()
                                + 24
                                + match &t.resolution {
                                    Resolution::Resolved { path } => 4 * path.len() + 8,
                                    Resolution::Unresolved(_) => 8,
                                }
                        })
                        .sum::<usize>()
            }
        }
    }
}

/// Where an element of the latest observation is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Spot {
    At(Vec<u32>),
    Conflict,
    /// Restored from a snapshot that kept labels only.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HandleEntry {
    pub label: String,
    pub spot: Spot,
}

pub(crate) type HandleMap = HashMap<String, HandleEntry>;

/// A 128-bit digest of the handle addresses of one observation (two FNV-1a passes over the
/// handles in sorted order, with the address of each). It is defined here and persisted, so it
/// does not depend on a hasher whose output may change between releases. Content interning
/// ignores handles, so this is what makes a retry with changed handles distinguishable.
pub(crate) fn handles_digest(m: &HandleMap) -> (u64, u64) {
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let (mut a, mut b) = (0xcbf2_9ce4_8422_2325u64, 0x8422_2325_cbf2_9ce4u64);
    let mut feed = |bytes: &[u8]| {
        for &x in bytes {
            a = (a ^ u64::from(x)).wrapping_mul(PRIME);
            b = (b ^ u64::from(x ^ 0x5a)).wrapping_mul(PRIME);
        }
    };
    let mut keys: Vec<&String> = m.keys().collect();
    keys.sort();
    for k in keys {
        feed(&(k.len() as u64).to_le_bytes());
        feed(k.as_bytes());
        match &m[k].spot {
            Spot::At(p) => {
                feed(&[0]);
                feed(&(p.len() as u64).to_le_bytes());
                for i in p {
                    feed(&i.to_le_bytes());
                }
            }
            Spot::Conflict => feed(&[1]),
            Spot::Unknown => feed(&[2]),
        }
    }
    (a, b)
}

pub(crate) fn handles_bound(m: &HandleMap) -> usize {
    m.iter()
        .map(|(h, e)| {
            h.len()
                + e.label.len()
                + 24
                + match &e.spot {
                    Spot::At(p) => 4 * p.len(),
                    _ => 0,
                }
        })
        .sum()
}

/// Every handle of `t` with its label and address. A handle on two elements is a conflict.
pub(crate) fn collect_handles(t: &TreeNode) -> HandleMap {
    let mut out = HandleMap::new();
    let mut put = |n: &TreeNode, path: Vec<u32>| {
        if let Some(h) = &n.handle {
            let spot = if out.contains_key(h) {
                Spot::Conflict
            } else {
                Spot::At(path)
            };
            out.insert(
                h.clone(),
                HandleEntry {
                    label: label(&n.role, &n.name),
                    spot,
                },
            );
        }
    };
    put(t, Vec::new());
    let mut frames: Vec<(&TreeNode, usize)> = vec![(t, 0)];
    while let Some(top) = frames.last_mut() {
        if top.1 < top.0.children.len() {
            let c = &top.0.children[top.1];
            top.1 += 1;
            let path: Vec<u32> = frames.iter().map(|f| (f.1 - 1) as u32).collect();
            put(c, path);
            frames.push((c, 0));
        } else {
            frames.pop();
        }
    }
    out
}

pub(crate) fn resolve(
    role: TargetRole,
    handle: &str,
    predecessor: Option<u32>,
    contiguous: bool,
    handles: Option<&HandleMap>,
) -> ActionTarget {
    let resolution = if predecessor.is_none() {
        Resolution::Unresolved(Unresolved::NoPredecessor)
    } else if !contiguous {
        Resolution::Unresolved(Unresolved::Discontinuity)
    } else {
        match handles.and_then(|m| m.get(handle)) {
            None => Resolution::Unresolved(Unresolved::HandleNotFound),
            Some(HandleEntry {
                spot: Spot::Conflict,
                ..
            }) => Resolution::Unresolved(Unresolved::HandleConflict),
            Some(HandleEntry {
                spot: Spot::Unknown,
                ..
            }) => Resolution::Unresolved(Unresolved::NoHandleContext),
            Some(HandleEntry {
                spot: Spot::At(p), ..
            }) => Resolution::Resolved { path: p.clone() },
        }
    };
    ActionTarget {
        role,
        handle: handle.to_string(),
        resolution,
    }
}

/// A durable reference to one occurrence: the episode's unique id (never reused, unlike its
/// storage slot), the step, and the child indices from that step's root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OccurrenceRef {
    pub episode_uid: u64,
    pub step: u32,
    pub path: Vec<u32>,
}

/// The node an occurrence reference addresses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OccurrenceNode {
    pub role: String,
    pub name: String,
    pub props: String,
}

/// An action with its predecessor and the occurrences it targeted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ActionTargets {
    pub record: ActionRecord,
    pub predecessor: Option<OccurrenceRef>,
    pub targets: Vec<(TargetRole, Option<OccurrenceRef>)>,
}
