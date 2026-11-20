//! Reusable procedures learned from episodes, and their revision by later observations.
//!
//! A procedure is the grounded action path (page template, `role 'name'` action) that at least
//! `min_support` successful episodes took. Each step expects what its supporters saw: the action
//! changes the page, shows no error and leads to the next step's page. An episode that takes the
//! same step and sees otherwise is a refutation, kept with its episode and step. Refutations
//! revise the abstraction instead of deleting it: when a control state held in every supporting
//! run and in no refuting run exists, the procedure becomes guarded by it; when none exists and
//! the refutations are as many as the support, it is retired. `ground` binds a step to a control
//! of a new page and checks the guard.

use super::form::Control;
use super::{Experience, PathEntry};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// The page the action is taken on.
    pub template: u32,
    pub page: String,
    pub action: String,
    /// The page the supporting runs reached.
    pub next: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefutationKind {
    NoChange,
    ErrorShown,
    WrongPage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refutation {
    pub episode: String,
    pub step: u32,
    pub kind: RefutationKind,
}

/// A control state that held before the step in every supporting run and in no refuting run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guard {
    pub step: usize,
    pub role: String,
    pub name: String,
    pub field: &'static str,
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Active,
    Revised,
    Retired,
}

#[derive(Clone, Debug)]
pub struct Procedure {
    pub goal: String,
    pub steps: Vec<Step>,
    pub support: Vec<String>,
    pub refutations: Vec<Refutation>,
    pub guard: Option<Guard>,
    pub status: Status,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Grounding {
    /// The step's control is on the page exactly once and the guard, if any, holds.
    Bound {
        role: String,
        name: String,
    },
    /// The control is on the page `n` times; the step does not identify one.
    Ambiguous(usize),
    Missing,
    /// The guard of the procedure does not hold on this page.
    GuardFails(Guard),
}

#[derive(Clone, Debug)]
pub struct ProcedureConfig {
    pub min_support: usize,
}

impl Default for ProcedureConfig {
    fn default() -> Self {
        Self { min_support: 2 }
    }
}

type Key = Vec<(u32, String, u32)>;

struct Run<'a> {
    episode: &'a str,
    path: Vec<PathEntry>,
}

fn key_of(path: &[PathEntry]) -> Key {
    (1..path.len())
        .filter_map(|i| {
            path[i]
                .action
                .clone()
                .map(|a| (path[i - 1].template.id, a, path[i].template.id))
        })
        .collect()
}

/// Index of the path entry produced by taking `st`'s action on `st`'s page.
fn locate(path: &[PathEntry], st: &Step) -> Option<usize> {
    (1..path.len()).find(|&i| {
        path[i].action.as_deref() == Some(&st.action) && path[i - 1].template.id == st.template
    })
}

fn fields(c: &Control) -> Vec<(&'static str, String)> {
    let mut v = Vec::new();
    let mut put = |f: &'static str, x: &Option<String>| {
        if let Some(x) = x {
            v.push((f, x.clone()));
        }
    };
    put("value", &c.value);
    put("checked", &c.checked);
    put("selected", &c.selected);
    put("expanded", &c.expanded);
    put("pressed", &c.pressed);
    if let Some(b) = c.required {
        v.push(("required", b.to_string()));
    }
    if let Some(b) = c.disabled {
        v.push(("disabled", b.to_string()));
    }
    v
}

fn holds(controls: &[Control], g: &Guard) -> bool {
    controls.iter().any(|c| {
        c.role == g.role
            && c.name == g.name
            && fields(c)
                .iter()
                .any(|(f, v)| *f == g.field && *v == g.value)
    })
}

#[derive(Default)]
pub struct ProcedureStore {
    cfg: ProcedureConfig,
    procs: Vec<Procedure>,
}

impl ProcedureStore {
    pub fn new(cfg: ProcedureConfig) -> Self {
        Self {
            cfg,
            procs: Vec::new(),
        }
    }

    pub fn procedures(&self) -> &[Procedure] {
        &self.procs
    }

    fn runs<'a>(&self, x: &Experience, ids: &'a [String]) -> Vec<Run<'a>> {
        ids.iter()
            .filter_map(|id| {
                x.episode(id).map(|e| Run {
                    episode: id.as_str(),
                    path: e.path,
                })
            })
            .collect()
    }

    /// Learn a procedure from every group of at least `min_support` successful episodes that took the
    /// same grounded path. Procedures already known are kept with their history.
    pub fn learn(&mut self, x: &Experience) {
        let ids = x.episode_ids();
        let mut groups: BTreeMap<Key, Vec<String>> = BTreeMap::new();
        for id in &ids {
            let Some(e) = x.episode(id) else { continue };
            if e.outcome != "success" {
                continue;
            }
            let k = key_of(&e.path);
            if k.len() >= 2 {
                groups.entry(k).or_default().push(id.clone());
            }
        }
        for (k, support) in groups {
            if support.len() < self.cfg.min_support {
                continue;
            }
            let steps: Vec<Step> = k
                .iter()
                .map(|(t, a, n)| Step {
                    template: *t,
                    page: x
                        .templates()
                        .into_iter()
                        .find(|i| i.id == *t)
                        .map(|i| i.title_template)
                        .unwrap_or_default(),
                    action: a.clone(),
                    next: *n,
                })
                .collect();
            match self.procs.iter_mut().find(|p| p.steps == steps) {
                Some(p) => {
                    for s in support {
                        if !p.support.contains(&s) {
                            p.support.push(s);
                        }
                    }
                }
                None => {
                    let goal = support
                        .iter()
                        .filter_map(|s| x.episode(s))
                        .map(|e| e.goal)
                        .min_by_key(|g| g.len())
                        .unwrap_or_default();
                    self.procs.push(Procedure {
                        goal,
                        steps,
                        support,
                        refutations: Vec::new(),
                        guard: None,
                        status: Status::Active,
                    });
                }
            }
        }
    }

    /// Check every stored episode against every procedure and revise the procedures an observation disproves.
    pub fn observe(&mut self, x: &Experience) {
        let ids = x.episode_ids();
        let runs = self.runs(x, &ids);
        for p in &mut self.procs {
            let mut seen: BTreeSet<(String, u32)> = p
                .refutations
                .iter()
                .map(|r| (r.episode.clone(), r.step))
                .collect();
            // (index of the step, episode, step number before the action) of every refuting traversal
            let mut bad: Vec<(usize, String, u32)> = Vec::new();
            for r in &runs {
                if p.support.iter().any(|s| s == r.episode) {
                    continue;
                }
                for (i, st) in p.steps.iter().enumerate() {
                    let Some(pos) = locate(&r.path, st) else {
                        continue;
                    };
                    let e = &r.path[pos];
                    let kind = if e.error {
                        Some(RefutationKind::ErrorShown)
                    } else if e.no_change {
                        Some(RefutationKind::NoChange)
                    } else if e.template.id != st.next {
                        Some(RefutationKind::WrongPage)
                    } else {
                        None
                    };
                    if let Some(kind) = kind {
                        if seen.insert((r.episode.to_string(), e.step)) {
                            p.refutations.push(Refutation {
                                episode: r.episode.to_string(),
                                step: e.step,
                                kind,
                            });
                        }
                        bad.push((i, r.episode.to_string(), r.path[pos - 1].step));
                    }
                }
            }
            if p.refutations.is_empty() {
                continue;
            }
            p.guard = Self::find_guard(x, p, &bad);
            p.status = if p.guard.is_some() {
                Status::Revised
            } else if p.refutations.len() >= p.support.len() {
                Status::Retired
            } else {
                Status::Active
            };
        }
    }

    /// A control state present before step `i` in every supporting run and in none of the refuting runs of that step.
    fn find_guard(x: &Experience, p: &Procedure, bad: &[(usize, String, u32)]) -> Option<Guard> {
        let controls = |ep: &str, step: u32| x.step_controls(ep, step).map(<[Control]>::to_vec);
        for (i, st) in p.steps.iter().enumerate() {
            let neg: Vec<Vec<Control>> = bad
                .iter()
                .filter(|b| b.0 == i)
                .filter_map(|b| controls(&b.1, b.2))
                .collect();
            let sup: Vec<Vec<Control>> = p
                .support
                .iter()
                .filter_map(|s| {
                    let e = x.episode(s)?;
                    let pos = locate(&e.path, st)?;
                    controls(s, e.path[pos - 1].step)
                })
                .collect();
            if sup.is_empty() || neg.is_empty() {
                continue;
            }
            let mut cands: BTreeSet<(String, String, &'static str, String)> = BTreeSet::new();
            for c in &sup[0] {
                for (f, v) in fields(c) {
                    cands.insert((c.role.clone(), c.name.clone(), f, v));
                }
            }
            for (role, name, field, value) in cands {
                let g = Guard {
                    step: i,
                    role,
                    name,
                    field,
                    value,
                };
                if sup.iter().all(|c| holds(c, &g)) && neg.iter().all(|c| !holds(c, &g)) {
                    return Some(g);
                }
            }
        }
        None
    }

    /// Bind step `i` of procedure `p` to a control of the current page.
    pub fn ground(p: &Procedure, i: usize, controls: &[Control]) -> Grounding {
        let Some(st) = p.steps.get(i) else {
            return Grounding::Missing;
        };
        if let Some(g) = p.guard.as_ref().filter(|g| g.step == i) {
            if !holds(controls, g) {
                return Grounding::GuardFails(g.clone());
            }
        }
        let hits: Vec<&Control> = controls
            .iter()
            .filter(|c| super::tree::label(&c.role, &c.name) == st.action)
            .collect();
        match hits.len() {
            0 => Grounding::Missing,
            1 => Grounding::Bound {
                role: hits[0].role.clone(),
                name: hits[0].name.clone(),
            },
            n => Grounding::Ambiguous(n),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tree::{label, TreeNode};
    use super::super::{Action, Experience, Observation};
    use super::*;

    fn page(title: &str, names: &[&str]) -> TreeNode {
        TreeNode::new("RootWebArea", title)
            .with_children(names.iter().map(|n| TreeNode::new("button", n)).collect())
    }

    fn reason(filled: bool) -> Control {
        Control {
            role: "textbox".into(),
            name: "Reason".into(),
            value: filled.then(|| "because".to_string()),
            ..Control::default()
        }
    }

    /// Orders -> click Open -> Detail -> click Save -> Saved. `blocked` makes Save leave the page unchanged.
    fn episode(x: &mut Experience, id: &str, filled: bool, blocked: bool) {
        x.describe_episode(id, "Save the order reason", "success")
            .unwrap();
        let orders = page("Orders", &["Open"]);
        let detail = page("Detail", &["Save"]);
        let saved = page("Saved", &["Done"]);
        let open = Action::Label(label("button", "Open"));
        let save = Action::Label(label("button", "Save"));
        let url = Some("http://h/orders/12");
        let step =
            |x: &mut Experience, n: u32, t: &TreeNode, a: Option<&Action>, c: Vec<Control>| {
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
        step(x, 0, &orders, None, vec![]);
        step(x, 1, &detail, Some(&open), vec![reason(filled)]);
        step(
            x,
            2,
            if blocked { &detail } else { &saved },
            Some(&save),
            vec![reason(filled)],
        );
    }

    #[test]
    fn learns_then_revises_with_a_guard_and_grounds() {
        let mut x = Experience::default();
        episode(&mut x, "e1", true, false);
        episode(&mut x, "e2", true, false);
        let mut s = ProcedureStore::default();
        s.learn(&x);
        assert_eq!(s.procedures().len(), 1);
        assert_eq!(s.procedures()[0].status, Status::Active);
        assert_eq!(s.procedures()[0].support.len(), 2);

        // An observation disproves "Save changes the page": same steps, empty Reason, nothing happens.
        episode(&mut x, "e3", false, true);
        s.observe(&x);
        let p = &s.procedures()[0];
        assert_eq!(p.refutations.len(), 1);
        assert_eq!(p.refutations[0].kind, RefutationKind::NoChange);
        assert_eq!(p.status, Status::Revised);
        let g = p.guard.as_ref().expect("a guard separates the runs");
        assert_eq!((g.name.as_str(), g.field), ("Reason", "value"));

        // The revision is stable when observed again, and keeps the disproving episode.
        s.observe(&x);
        assert_eq!(s.procedures()[0].refutations.len(), 1);

        let save_step = s.procedures()[0].steps.len() - 1;
        let save_btn = Control {
            role: "button".into(),
            name: "Save".into(),
            ..Control::default()
        };
        assert_eq!(
            ProcedureStore::ground(
                &s.procedures()[0],
                save_step,
                &[reason(true), save_btn.clone()]
            ),
            Grounding::Bound {
                role: "button".into(),
                name: "Save".into()
            }
        );
        assert!(matches!(
            ProcedureStore::ground(
                &s.procedures()[0],
                save_step,
                &[reason(false), save_btn.clone()]
            ),
            Grounding::GuardFails(_)
        ));
        assert_eq!(
            ProcedureStore::ground(
                &s.procedures()[0],
                save_step,
                &[reason(true), save_btn.clone(), save_btn]
            ),
            Grounding::Ambiguous(2)
        );
        assert_eq!(
            ProcedureStore::ground(&s.procedures()[0], save_step, &[reason(true)]),
            Grounding::Missing
        );
    }

    #[test]
    fn retires_when_nothing_separates_refutations_from_support() {
        let mut x = Experience::default();
        episode(&mut x, "e1", true, false);
        episode(&mut x, "e2", true, false);
        let mut s = ProcedureStore::default();
        s.learn(&x);
        // Refuting runs look exactly like the supporting ones before the step.
        episode(&mut x, "e3", true, true);
        episode(&mut x, "e4", true, true);
        s.observe(&x);
        assert_eq!(s.procedures()[0].refutations.len(), 2);
        assert!(s.procedures()[0].guard.is_none());
        assert_eq!(s.procedures()[0].status, Status::Retired);
    }

    #[test]
    fn one_supporter_is_not_a_procedure() {
        let mut x = Experience::default();
        episode(&mut x, "e1", true, false);
        let mut s = ProcedureStore::default();
        s.learn(&x);
        assert!(s.procedures().is_empty());
    }
}
