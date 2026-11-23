// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Conditions under which an action takes effect, learned as a version space.
//!
//! Every observed transition (the controls before an action, and whether the action had an effect)
//! is evidence. The hypothesis class is explicit and bounded: conjunctions of at most two atoms over
//! the page's controls (the target is not disabled; at least or exactly k checked controls of a
//! role; all of a role checked; a field is filled; a control state equals a value). A hypothesis
//! survives while it agrees with every observation. A prediction is settled only when every
//! survivor agrees; otherwise it is unresolved and names the alternatives. The model is a function
//! of the stored episodes, so purging an episode reopens exactly the alternatives it had excluded.
//!
//! Limits: an action has an effect when it shows no error and changes the page or its state; a role group is every control of that role on the page, not a region; atoms are positive
//! only; an effect that depends on state the controls do not carry leaves no surviving hypothesis,
//! reported as `Unexplained`.

use super::form::Control;
use super::procedures::fields;
use super::Experience;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Atom {
    /// The control the action targets is not disabled.
    TargetEnabled,
    AtLeast {
        role: String,
        k: u32,
    },
    Exactly {
        role: String,
        k: u32,
    },
    All {
        role: String,
    },
    Filled {
        role: String,
        name: String,
    },
    Equals {
        role: String,
        name: String,
        field: &'static str,
        value: String,
    },
}

/// A conjunction of atoms; the empty conjunction says the action always has an effect.
pub type Hypothesis = Vec<Atom>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evidence {
    pub episode: String,
    pub step: u32,
    pub effective: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// No observation of this action carried controls.
    NoEvidence,
    /// No hypothesis of the class fits the observations.
    Unexplained,
    /// Every surviving hypothesis predicts an effect. The witnesses are the observations that
    /// excluded the alternatives predicting otherwise.
    Effective {
        witnesses: Vec<Evidence>,
    },
    Ineffective {
        witnesses: Vec<Evidence>,
    },
    /// Survivors disagree; the simplest are listed.
    Unresolved {
        alternatives: Vec<Hypothesis>,
    },
}

#[derive(Clone, Debug)]
pub struct EnablementConfig {
    pub max_atoms: usize,
    pub max_witnesses: usize,
    pub max_alternatives: usize,
}

impl Default for EnablementConfig {
    fn default() -> Self {
        Self {
            max_atoms: 96,
            max_witnesses: 8,
            max_alternatives: 8,
        }
    }
}

pub struct Model {
    pub template: u32,
    pub action: String,
    target: Option<(String, String)>,
    evidence: Vec<Evidence>,
    survivors: Vec<Hypothesis>,
    /// A hypothesis the evidence excluded, with the index of the first observation that did.
    excluded: Vec<(Hypothesis, usize)>,
}

type Observations = Vec<(Evidence, Vec<Control>)>;

#[derive(Default)]
pub struct EnablementStore {
    cfg: EnablementConfig,
    models: BTreeMap<(u32, String), Model>,
}

/// The role and name of a `role 'name'` action label.
fn target_of(action: &str) -> Option<(String, String)> {
    let (role, rest) = action.split_once(' ')?;
    let name = rest.strip_prefix('\'')?.strip_suffix('\'')?;
    Some((role.to_string(), name.to_string()))
}

fn is_on(c: &Control) -> bool {
    [&c.checked, &c.selected, &c.pressed]
        .iter()
        .any(|f| f.as_deref().is_some_and(|v| v.eq_ignore_ascii_case("true")))
}

fn checkable(c: &Control) -> bool {
    c.checked.is_some() || c.selected.is_some() || c.pressed.is_some()
}

fn counts(state: &[Control], role: &str) -> (u32, u32) {
    let (mut on, mut total) = (0, 0);
    for c in state.iter().filter(|c| c.role == role && checkable(c)) {
        total += 1;
        on += u32::from(is_on(c));
    }
    (on, total)
}

fn holds(a: &Atom, state: &[Control], target: &Option<(String, String)>) -> bool {
    match a {
        Atom::TargetEnabled => match target {
            Some((r, n)) => !state
                .iter()
                .any(|c| &c.role == r && &c.name == n && c.disabled == Some(true)),
            None => true,
        },
        Atom::AtLeast { role, k } => counts(state, role).0 >= *k,
        Atom::Exactly { role, k } => counts(state, role).0 == *k,
        Atom::All { role } => {
            let (on, total) = counts(state, role);
            total > 0 && on == total
        }
        Atom::Filled { role, name } => state.iter().any(|c| {
            &c.role == role
                && &c.name == name
                && c.value.as_deref().is_some_and(|v| !v.trim().is_empty())
        }),
        Atom::Equals {
            role,
            name,
            field,
            value,
        } => state.iter().any(|c| {
            &c.role == role
                && &c.name == name
                && fields(c).iter().any(|(f, v)| f == field && v == value)
        }),
    }
}

fn atoms_of(states: &[&[Control]], cap: usize) -> Vec<Atom> {
    let mut set: BTreeSet<Atom> = BTreeSet::new();
    set.insert(Atom::TargetEnabled);
    for state in states {
        for c in state.iter() {
            if checkable(c) {
                for k in [1, 2] {
                    set.insert(Atom::AtLeast {
                        role: c.role.clone(),
                        k,
                    });
                    set.insert(Atom::Exactly {
                        role: c.role.clone(),
                        k,
                    });
                }
                set.insert(Atom::All {
                    role: c.role.clone(),
                });
            }
            if c.role == "button" {
                continue;
            }
            if c.value.is_some() {
                set.insert(Atom::Filled {
                    role: c.role.clone(),
                    name: c.name.clone(),
                });
            }
            for (field, value) in fields(c) {
                if field != "value" || value.chars().count() <= 40 {
                    set.insert(Atom::Equals {
                        role: c.role.clone(),
                        name: c.name.clone(),
                        field,
                        value,
                    });
                }
            }
        }
    }
    set.into_iter().take(cap).collect()
}

fn hypotheses(atoms: &[Atom]) -> Vec<Vec<usize>> {
    let mut h: Vec<Vec<usize>> = vec![vec![]];
    h.extend((0..atoms.len()).map(|i| vec![i]));
    for i in 0..atoms.len() {
        for j in i + 1..atoms.len() {
            h.push(vec![i, j]);
        }
    }
    h
}

impl Model {
    fn build(cfg: &EnablementConfig, template: u32, action: &str, obs: Observations) -> Self {
        let target = target_of(action);
        let states: Vec<&[Control]> = obs.iter().map(|(_, s)| s.as_slice()).collect();
        let atoms = atoms_of(&states, cfg.max_atoms);
        let truth: Vec<Vec<bool>> = obs
            .iter()
            .map(|(_, s)| atoms.iter().map(|a| holds(a, s, &target)).collect())
            .collect();
        let mut survivors = Vec::new();
        let mut excluded = Vec::new();
        for h in hypotheses(&atoms) {
            let wrong = obs
                .iter()
                .enumerate()
                .position(|(o, (e, _))| h.iter().all(|&i| truth[o][i]) != e.effective);
            let atoms_of_h: Hypothesis = h.iter().map(|&i| atoms[i].clone()).collect();
            match wrong {
                None => survivors.push(atoms_of_h),
                Some(o) => excluded.push((atoms_of_h, o)),
            }
        }
        Self {
            template,
            action: action.to_string(),
            target,
            evidence: obs.into_iter().map(|(e, _)| e).collect(),
            survivors,
            excluded,
        }
    }

    pub fn evidence(&self) -> &[Evidence] {
        &self.evidence
    }

    pub fn survivors(&self) -> &[Hypothesis] {
        &self.survivors
    }

    fn says(&self, h: &Hypothesis, state: &[Control]) -> bool {
        h.iter().all(|a| holds(a, state, &self.target))
    }

    pub fn predict(&self, cfg: &EnablementConfig, state: &[Control]) -> Verdict {
        if self.evidence.is_empty() {
            return Verdict::NoEvidence;
        }
        if self.survivors.is_empty() {
            return Verdict::Unexplained;
        }
        let yes = self
            .survivors
            .iter()
            .filter(|h| self.says(h, state))
            .count();
        let verdict = if yes == self.survivors.len() {
            Some(true)
        } else if yes == 0 {
            Some(false)
        } else {
            None
        };
        let Some(effective) = verdict else {
            let mut alts = self.survivors.clone();
            alts.sort_by_key(Vec::len);
            alts.truncate(cfg.max_alternatives);
            return Verdict::Unresolved { alternatives: alts };
        };
        let mut idx: BTreeSet<usize> = BTreeSet::new();
        for (h, o) in &self.excluded {
            if self.says(h, state) != effective {
                idx.insert(*o);
            }
        }
        let witnesses = idx
            .into_iter()
            .take(cfg.max_witnesses)
            .map(|o| self.evidence[o].clone())
            .collect();
        if effective {
            Verdict::Effective { witnesses }
        } else {
            Verdict::Ineffective { witnesses }
        }
    }
}

impl EnablementStore {
    pub fn new(cfg: EnablementConfig) -> Self {
        Self {
            cfg,
            models: BTreeMap::new(),
        }
    }

    /// Recompute every model from the episodes now stored.
    pub fn refresh(&mut self, x: &Experience) {
        let mut groups: BTreeMap<(u32, String), Observations> = BTreeMap::new();
        for id in x.episode_ids() {
            let Some(e) = x.episode(&id) else { continue };
            for i in 1..e.path.len() {
                let (Some(action), prev) = (&e.path[i].action, &e.path[i - 1]) else {
                    continue;
                };
                let Some(state) = x.step_controls(&id, prev.step) else {
                    continue;
                };
                if state.is_empty() {
                    continue;
                }
                let st = &e.path[i];
                let effective = !st.error && (!st.no_change || st.state_changed);
                groups
                    .entry((prev.template.id, action.clone()))
                    .or_default()
                    .push((
                        Evidence {
                            episode: id.clone(),
                            step: st.step,
                            effective,
                        },
                        state.to_vec(),
                    ));
            }
        }
        self.models = groups
            .into_iter()
            .map(|((t, a), obs)| {
                let m = Model::build(&self.cfg, t, &a, obs);
                ((t, a), m)
            })
            .collect();
    }

    pub fn learn(x: &Experience) -> Self {
        let mut s = Self::default();
        s.refresh(x);
        s
    }

    pub fn model(&self, template: u32, action: &str) -> Option<&Model> {
        self.models.get(&(template, action.to_string()))
    }

    pub fn predict(&self, template: u32, action: &str, state: &[Control]) -> Verdict {
        match self.model(template, action) {
            Some(m) => m.predict(&self.cfg, state),
            None => Verdict::NoEvidence,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tree::{label, TreeNode};
    use super::super::{Action, Experience, Observation};
    use super::*;

    fn boxed(name: &str, on: bool) -> Control {
        Control {
            role: "checkbox".into(),
            name: name.into(),
            checked: Some(on.to_string()),
            ..Control::default()
        }
    }

    fn save(disabled: bool) -> Control {
        Control {
            role: "button".into(),
            name: "Save".into(),
            disabled: Some(disabled),
            ..Control::default()
        }
    }

    fn state(on: [bool; 3], disabled: bool) -> Vec<Control> {
        vec![
            boxed("A", on[0]),
            boxed("B", on[1]),
            boxed("C", on[2]),
            save(disabled),
        ]
    }

    /// One transition: Save is clicked on a form in `state`; the page changes iff `effective`.
    fn observe(x: &mut Experience, id: &str, st: Vec<Control>, effective: bool) {
        x.describe_episode(id, "Save", "success").unwrap();
        let form = TreeNode::new("RootWebArea", "Form")
            .with_children(vec![TreeNode::new("button", "Save")]);
        let done = TreeNode::new("RootWebArea", "Saved");
        let click = Action::Label(label("button", "Save"));
        let url = Some("http://h/form");
        let mut put = |n, t: &TreeNode, a, c| {
            x.ingest_with_controls(
                &Observation {
                    episode: id,
                    step: n,
                    tree: t,
                    url,
                    action: a,
                },
                c,
            )
            .unwrap();
        };
        put(0, &form, None, st.clone());
        put(1, if effective { &done } else { &form }, Some(&click), st);
    }

    fn form_template(x: &Experience) -> u32 {
        x.template_id(Some("http://h/form"), "Form").unwrap()
    }

    fn corpus(x: &mut Experience, with_two: bool) {
        observe(x, "none", state([false; 3], false), false);
        observe(x, "one", state([true, false, false], false), true);
        if with_two {
            observe(x, "two", state([false, true, true], false), true);
        }
    }

    fn verdict(x: &Experience, on: [bool; 3], disabled: bool) -> Verdict {
        let t = form_template(x);
        EnablementStore::learn(x).predict(t, "button 'Save'", &state(on, disabled))
    }

    #[test]
    fn learns_the_gating_condition_and_names_the_excluding_observations() {
        let mut x = Experience::default();
        corpus(&mut x, true);
        match verdict(&x, [true, true, true], false) {
            Verdict::Effective { witnesses } => {
                let eps: Vec<&str> = witnesses.iter().map(|w| w.episode.as_str()).collect();
                assert!(eps.contains(&"two"), "{eps:?}");
            }
            v => panic!("{v:?}"),
        }
        assert!(matches!(
            verdict(&x, [false; 3], false),
            Verdict::Ineffective { .. }
        ));
    }

    #[test]
    fn withdrawing_the_distinguishing_observation_reopens_the_alternative() {
        let mut x = Experience::default();
        corpus(&mut x, true);
        // Two boxes checked: only "at least one" survives, so it is settled.
        assert!(matches!(
            verdict(&x, [false, true, true], false),
            Verdict::Effective { .. }
        ));
        assert!(x.purge_episode("two").is_some());
        // Without it "exactly one" is back, and two boxes no longer settle the question.
        match verdict(&x, [false, true, true], false) {
            Verdict::Unresolved { alternatives } => {
                assert!(alternatives
                    .iter()
                    .any(|h| matches!(h.as_slice(), [Atom::Exactly { k: 1, .. }])));
            }
            v => panic!("{v:?}"),
        }
        // The contract: retract then answer equals learn on the remaining evidence.
        let mut fresh = Experience::default();
        corpus(&mut fresh, false);
        for on in [
            [true; 3],
            [false, true, true],
            [false; 3],
            [true, false, false],
        ] {
            assert_eq!(verdict(&x, on, false), verdict(&fresh, on, false));
        }
    }

    #[test]
    fn a_disabled_target_is_learned_from_the_page_state() {
        let mut x = Experience::default();
        observe(&mut x, "off", state([true, true, true], true), false);
        observe(&mut x, "on", state([true, true, true], false), true);
        observe(&mut x, "idle", state([false; 3], false), false);
        assert!(matches!(
            verdict(&x, [true, true, true], true),
            Verdict::Ineffective { .. }
        ));
        assert!(matches!(
            verdict(&x, [true, true, true], false),
            Verdict::Effective { .. }
        ));
        // One checked box is not covered by the evidence: "at least 1" and "at least 2" both fit.
        assert!(matches!(
            verdict(&x, [true, false, false], false),
            Verdict::Unresolved { .. }
        ));
    }

    #[test]
    fn an_effect_the_controls_cannot_explain_leaves_no_hypothesis() {
        let mut x = Experience::default();
        observe(&mut x, "a", state([true, false, false], false), true);
        observe(&mut x, "b", state([true, false, false], false), false);
        assert_eq!(
            verdict(&x, [true, false, false], false),
            Verdict::Unexplained
        );
        let empty = Experience::default();
        assert_eq!(
            EnablementStore::learn(&empty).predict(0, "button 'Save'", &[]),
            Verdict::NoEvidence
        );
    }
}
