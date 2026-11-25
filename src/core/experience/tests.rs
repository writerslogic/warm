use super::tree::parse_indented;
use super::*;
use serde_json::json;
use std::collections::BTreeMap;

fn page(title: &str, body: Vec<TreeNode>) -> TreeNode {
    TreeNode::new("RootWebArea", title).with_children(body)
}

fn btn(name: &str) -> TreeNode {
    TreeNode::new("button", name)
}

fn obs<'a>(
    episode: &'a str,
    step: u32,
    tree: &'a TreeNode,
    action: Option<&'a Action>,
) -> Observation<'a> {
    Observation {
        episode,
        step,
        tree,
        url: Some("http://h/orders/12"),
        action,
    }
}

fn click(label: &str) -> Action {
    Action::Label(label.to_string())
}

fn chain(depth: usize) -> TreeNode {
    let mut t = TreeNode::new("leaf", "x");
    for _ in 1..depth {
        t = TreeNode::new("group", "g").with_children(vec![t]);
    }
    t
}

fn wide(nodes: usize) -> TreeNode {
    page("w", (1..nodes).map(|i| btn(&format!("b{i}"))).collect())
}

fn episode_a() -> Vec<TreeNode> {
    vec![
        page("Orders", vec![btn("Open"), btn("Filter")]),
        page("Detail", vec![btn("Edit"), btn("Back")]),
        page(
            "Detail",
            vec![
                btn("Edit"),
                btn("Back"),
                TreeNode::new("alert", "Error: failed to save"),
            ],
        ),
    ]
}

fn run(x: &mut Experience, id: &str, trees: &[TreeNode]) {
    for (i, t) in trees.iter().enumerate() {
        let a = click(&format!("click button 'a{i}'"));
        x.ingest(&obs(id, i as u32, t, (i > 0).then_some(&a)))
            .unwrap();
    }
}

#[test]
fn equal_trees_intern_to_equal_ids_and_different_ones_do_not() {
    let mut t = TermStore::default();
    let a = page("P", vec![btn("Save"), btn("Cancel")]);
    let (x, y) = (t.intern(&a), t.intern(&a.clone()));
    assert_eq!(x, y);
    assert_eq!(t.live_nodes(), 3);
    let other = page("P", vec![btn("Save"), btn("Cancel!")]);
    assert_ne!(t.intern(&other), x);
    let mut checked = a.clone();
    checked.children[0].props = "checked".into();
    let z = t.intern(&checked);
    assert_ne!(z, x, "content ids differ when state differs");
    assert_eq!(t.node(z).sid, t.node(x).sid, "structural ids ignore state");
    let mut handled = a.clone();
    handled.children[0].handle = Some("7".into());
    assert_eq!(t.intern(&handled), x, "handles are not identity");
}

#[test]
fn repeated_pages_share_structure() {
    let mut x = Experience::default();
    let pages = [
        page("A", (0..30).map(|i| btn(&format!("a{i}"))).collect()),
        page("B", (0..30).map(|i| btn(&format!("b{i}"))).collect()),
        page("C", (0..30).map(|i| btn(&format!("c{i}"))).collect()),
    ];
    let raw_nodes = 200 * 31;
    for i in 0..200u32 {
        let a = click("click");
        x.ingest(&obs("e", i, &pages[i as usize % 3], (i > 0).then_some(&a)))
            .unwrap();
    }
    let s = x.stats();
    assert_eq!(s.steps, 200);
    assert_eq!(s.nodes, 93, "3 pages of 31 nodes");
    assert!(s.nodes * 50 < raw_nodes);
    assert_eq!(s.frames, 3);
    assert!(s.regions as u64 * 20 < s.region_occurrences);
}

#[test]
fn effect_updates_are_idempotent_per_step() {
    let mut x = Experience::default();
    let trees = episode_a();
    run(&mut x, "e1", &trees);
    let before = x.stats();
    let tid = x.template_id(Some("http://h/orders/12"), "Orders").unwrap();
    let eff = x.effects(tid, None, 10);
    assert_eq!(eff.len(), 1);
    assert_eq!(eff[0].count, 1);
    let a = click("click button 'a1'");
    assert_eq!(
        x.ingest(&obs("e1", 1, &trees[1], Some(&a))).unwrap(),
        Ingested::Duplicate
    );
    assert_eq!(x.stats(), before);
    assert_eq!(x.effects(tid, None, 10), eff);
    assert!(matches!(
        x.ingest(&obs("e1", 1, &trees[2], Some(&a))),
        Err(ExperienceError::Conflict { .. })
    ));
    assert_eq!(x.stats(), before, "a rejected step leaves no trace");
    let b = click("click");
    assert!(matches!(
        x.ingest(&obs("e1", 0, &trees[1], Some(&b))),
        Err(ExperienceError::Conflict { .. })
    ));
    let err = x.ingest(&obs("e1", 5, &trees[2], Some(&a))).unwrap();
    assert_eq!(err, Ingested::Applied);
    assert!(matches!(
        x.ingest(&obs("e1", 3, &trees[2], Some(&a))),
        Err(ExperienceError::OutOfOrder { .. })
    ));
}

#[test]
fn effect_rows_record_changes_loops_and_errors() {
    let mut x = Experience::default();
    let trees = episode_a();
    let same = trees[2].clone();
    run(&mut x, "e1", &trees);
    let a = click("click button 'a2'");
    x.ingest(&obs("e1", 3, &same, Some(&a))).unwrap();
    let detail = x.template_id(Some("http://h/orders/12"), "Detail").unwrap();
    let rows = x.effects(detail, Some("click button 'a2'"), 5);
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].count, rows[0].self_loops), (2, 1));
    let rows = x.effects(detail, None, 5);
    assert!(rows.iter().all(|r| r.template.id == detail));
    let orders = x.template_id(Some("http://h/orders/12"), "Orders").unwrap();
    let r = &x.effects(orders, None, 5)[0];
    assert_eq!(r.next[0].0.title_template, "Detail");
    assert!(r.added.iter().any(|(l, _)| l == "button 'Edit'"));
    assert!(r.removed.iter().any(|(l, _)| l == "button 'Open'"));
    let r2 = &x.effects(detail, Some("click button 'a2'"), 5)[0];
    assert!(r2.render().contains("nondeterministic: 1 of 2 no change"));
    let path = x.episode("e1").unwrap().path;
    assert!(path[2].error && !path[2].no_change);
    assert!(path[3].no_change);
}

#[test]
fn purge_frees_exactly_the_unshared_state() {
    let shared = page("Shared", vec![btn("Go"), btn("Stop")]);
    let only_a = page("OnlyA", vec![btn("alpha one"), btn("alpha two")]);
    let only_b = page("OnlyB", vec![btn("beta one"), btn("beta two")]);
    let ta = vec![shared.clone(), only_a];
    let tb = vec![shared, only_b];

    let mut both = Experience::default();
    run(&mut both, "a", &ta);
    run(&mut both, "b", &tb);
    let mut just_b = Experience::default();
    run(&mut just_b, "b", &tb);
    let mut just_a = Experience::default();
    run(&mut just_a, "a", &ta);
    let (full, b_only) = (both.stats(), just_b.stats());
    assert!(full.nodes > b_only.nodes);

    let report = both.purge_episode("a").unwrap();
    assert_eq!(both.stats(), b_only, "only the unshared state is gone");
    assert_eq!(report.nodes_freed, full.nodes - b_only.nodes);
    assert_eq!(report.steps, 2);
    assert!(both.search_regions("alpha", 5).unwrap().is_empty());
    assert!(!both.search_regions("beta", 5).unwrap().is_empty());
    assert!(
        !both.search_regions("Stop", 5).unwrap().is_empty(),
        "shared region survives with refcount 1"
    );
    assert_eq!(just_a.purge_episode("nope"), None);

    both.purge_episode("b").unwrap();
    assert_eq!(
        both.stats(),
        ExperienceStats::default(),
        "refcount 0 frees everything"
    );
    assert_eq!(both.terms.live_structs(), 0);
    assert!(both.search_regions("Stop", 5).unwrap().is_empty());
    assert_eq!(both.purge_episode("b"), None);
}

#[test]
fn purge_of_a_shared_region_keeps_remaining_provenance() {
    let mut x = Experience::default();
    let t = page("P", vec![btn("Shared button")]);
    run(&mut x, "a", std::slice::from_ref(&t));
    run(&mut x, "b", std::slice::from_ref(&t));
    run(&mut x, "c", &[t]);
    let hits = x.search_regions("shared button", 5).unwrap();
    assert_eq!((hits[0].episodes, hits[0].provenance.len()), (3, 3));
    x.purge_episode("b").unwrap();
    let hits = x.search_regions("shared button", 5).unwrap();
    let eps: Vec<&str> = hits[0]
        .provenance
        .iter()
        .map(|p| p.episode.as_str())
        .collect();
    assert_eq!(eps, ["a", "c"]);
}

#[test]
fn limits_hold_at_limit_minus_one_limit_and_plus_one() {
    let cfg = ExperienceConfig {
        max_depth: 20,
        max_nodes: 50,
        max_text_bytes: 64,
        max_id_bytes: 8,
        max_steps: 3,
        ..ExperienceConfig::default()
    };
    let mut x = Experience::new(cfg);
    for (d, ok) in [(19, true), (20, true), (21, false)] {
        let t = chain(d);
        assert_eq!(
            x.ingest(&obs("d", d as u32, &t, None)).is_ok(),
            ok,
            "depth {d}"
        );
    }
    for (n, ok) in [(49, true), (50, true), (51, false)] {
        let t = wide(n);
        assert_eq!(
            x.ingest(&obs("n", n as u32, &t, None)).is_ok(),
            ok,
            "nodes {n}"
        );
    }
    for (len, ok) in [(63, true), (64, true), (65, false)] {
        let t = page("s", vec![btn(&"x".repeat(len))]);
        assert_eq!(
            x.ingest(&obs("s", len as u32, &t, None)).is_ok(),
            ok,
            "string {len}"
        );
    }
    let t = btn("b");
    for (len, ok) in [(7, true), (8, true), (9, false)] {
        assert_eq!(
            x.ingest(&obs(&"i".repeat(len), 0, &t, None)).is_ok(),
            ok,
            "id {len}"
        );
    }
    for (i, ok) in [(0, true), (1, true), (2, true), (3, false)] {
        assert_eq!(x.ingest(&obs("steps", i, &t, None)).is_ok(), ok, "step {i}");
    }
    assert!(x.ingest(&obs("", 0, &t, None)).is_err());
    let rejected = Experience::new(ExperienceConfig::default());
    assert_eq!(rejected.stats(), ExperienceStats::default());
    let mut y = Experience::new(ExperienceConfig {
        max_nodes: 5,
        ..Default::default()
    });
    let before = y.stats();
    assert!(y.ingest(&obs("e", 0, &wide(6), None)).is_err());
    assert_eq!(y.stats(), before, "a rejected observation leaves no trace");
}

#[test]
fn indented_parser_reads_names_handles_and_state() {
    let text = "RootWebArea 'Home'\n\t[3] button 'It\\'s' disabled\n\t\t[4] textbox 'q' value='a b' required checked=true level=2\n\tnonsense line";
    let t = parse_indented(text).unwrap();
    assert_eq!((t.role.as_str(), t.name.as_str()), ("RootWebArea", "Home"));
    assert_eq!(t.children[0].name, "It's");
    assert_eq!(t.children[0].handle.as_deref(), Some("3"));
    assert_eq!(t.children[0].props, "disabled");
    assert_eq!(
        t.children[0].children[0].props,
        "value='a b' required checked=true level=2"
    );
    assert_eq!(t.children[1].role, "nonsense");
    assert!(parse_indented("  \n\n").is_none());
}

#[test]
fn detection_routes_by_shape_and_hits_become_document_results() {
    let v = json!({"episode": "e", "step": 0, "tree": {"role": "RootWebArea", "name": "Home",
        "children": [{"role": "button", "name": "Buy now"}]}});
    let o = detect_observation(&v).unwrap();
    assert!(detect_observation(&json!({"text": "plain memory"})).is_none());
    assert!(detect_observation(&json!({"episode": "e", "step": "x", "tree": {}})).is_none());
    let mut x = Experience::default();
    x.ingest(&o.as_observation()).unwrap();
    let hit = x.search_regions("buy now", 3).unwrap().remove(0);
    let d = hit.into_document_result();
    assert_eq!(d.metadata["kind"], "experience-region");
    assert_eq!(d.metadata["provenance"][0]["episode"], "e");
    assert!(d.text.unwrap().contains("Buy now"));
}

#[test]
fn episodes_list_by_goal_text() {
    let mut x = Experience::default();
    x.describe_episode("e1", "Cancel the pending order for Mia", "success")
        .unwrap();
    x.describe_episode("e2", "Export the monthly revenue report", "failure")
        .unwrap();
    run(&mut x, "e1", &episode_a());
    run(&mut x, "e2", &episode_a()[..2]);
    let hits = x.list_episodes("cancel pending order", 5).unwrap();
    assert_eq!(hits[0].episode, "e1");
    assert_eq!(hits[0].steps, 3);
    assert_eq!(
        x.list_episodes("revenue report", 5).unwrap()[0].outcome,
        "failure"
    );
    x.purge_episode("e1").unwrap();
    assert!(x
        .list_episodes("cancel pending order", 5)
        .unwrap()
        .iter()
        .all(|h| h.episode != "e1"));
}

fn fingerprint(x: &mut Experience) -> serde_json::Value {
    let mut eff = Vec::new();
    for t in x.templates() {
        for e in x.effects(t.id, None, 100) {
            eff.push(e.render());
        }
    }
    let regions: Vec<String> = x
        .search_regions("button portal order", 50)
        .unwrap()
        .iter()
        .map(|h| format!("{:.6} {}", h.score, h.text))
        .collect();
    let eps = x.list_episodes("gift order", 10).unwrap();
    json!({"stats": format!("{:?}", x.stats()), "effects": eff, "regions": regions,
           "episodes": eps.iter().map(|e| (e.episode.clone(), e.score.to_bits())).collect::<Vec<_>>(),
           "paths": x.episode("t0").map(|e| format!("{:?}", e.path))})
}

#[test]
fn snapshot_round_trips_and_rejects_damage() {
    let mut x = parity_store();
    x.purge_episode("t1").unwrap();
    let bytes = x.to_bytes().unwrap();
    let mut y = Experience::from_bytes(&bytes, ExperienceConfig::default()).unwrap();
    assert_eq!(fingerprint(&mut x), fingerprint(&mut y));
    assert_eq!(
        y.to_bytes().unwrap(),
        bytes,
        "reload then save is byte-identical"
    );
    let a = click("click");
    let t = page("new", vec![btn("n")]);
    y.ingest(&obs("t0", 99, &t, Some(&a))).unwrap();
    y.purge_episode("t0").unwrap();
    y.purge_episode("t2").unwrap();
    y.purge_episode("t3").unwrap();
    assert_eq!(
        y.stats(),
        ExperienceStats::default(),
        "a reloaded store purges to empty"
    );

    let mut flipped = bytes.clone();
    let last = flipped.len() - 1;
    flipped[last] ^= 0x40;
    assert!(Experience::from_bytes(&flipped, ExperienceConfig::default()).is_err());
    assert!(
        Experience::from_bytes(&bytes[..bytes.len() - 1], ExperienceConfig::default()).is_err()
    );
    let mut future = bytes.clone();
    future[4] = 6;
    assert!(Experience::from_bytes(&future, ExperienceConfig::default()).is_err());
    assert!(Experience::from_bytes(b"WEXP", ExperienceConfig::default()).is_err());

    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        Experience::open(dir.path(), ExperienceConfig::default())
            .unwrap()
            .stats(),
        ExperienceStats::default()
    );
    x.save(dir.path()).unwrap();
    let mut z = Experience::open(dir.path(), ExperienceConfig::default()).unwrap();
    assert_eq!(fingerprint(&mut x), fingerprint(&mut z));
    std::fs::write(dir.path().join(SNAPSHOT_FILE), b"garbage").unwrap();
    assert!(Experience::open(dir.path(), ExperienceConfig::default()).is_err());
}

#[test]
fn snapshot_with_a_node_cycle_is_rejected() {
    let mut x = Experience::default();
    run(&mut x, "e", &[page("P", vec![btn("a")])]);
    let good = x.to_bytes().unwrap();
    assert!(Experience::from_bytes(&good, ExperienceConfig::default()).is_ok());
    let mut snap: super::snapshot::Snap = ciborium::from_reader(&good[20..]).unwrap();
    let n = snap
        .nodes
        .iter()
        .position(|n| n.as_ref().is_some_and(|n| !n.3.is_empty()))
        .unwrap();
    snap.nodes[n].as_mut().unwrap().3 = vec![n as u32];
    assert!(Experience::rebuild(snap, ExperienceConfig::default()).is_err());
}

type UnitKey = (String, String, u64, usize, BTreeMap<String, u32>);

#[derive(Deserialize)]
struct Fixture {
    trajectories: Vec<Traj>,
    units: Vec<UnitRow>,
    effects: Vec<EffectRow>,
    templates: Vec<TemplateRow>,
    episodes: BTreeMap<String, String>,
}
#[derive(Deserialize)]
struct Traj {
    id: String,
    goal: String,
    outcome: String,
    states: Vec<State>,
}
#[derive(Deserialize)]
struct State {
    url: String,
    action: Option<String>,
    accessibility_tree: String,
    state_index: u32,
}
#[derive(Deserialize)]
struct UnitRow {
    text: String,
    breadcrumb: String,
    occurrences: u64,
    episodes: usize,
    values: BTreeMap<String, u32>,
}
#[derive(Deserialize)]
struct EffectRow {
    template: String,
    action: String,
    n: u32,
    loops: u32,
    add: BTreeMap<String, u32>,
    rem: BTreeMap<String, u32>,
    next: BTreeMap<String, u32>,
    err: BTreeMap<String, u32>,
}
#[derive(Deserialize)]
struct TemplateRow {
    template: String,
    obs: u32,
    inventory: BTreeMap<String, u32>,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/experience/parity_fixture.json"
    ))
    .unwrap()
}

fn parity_store() -> Experience {
    let mut x = Experience::default();
    for t in fixture().trajectories {
        x.describe_episode(&t.id, &t.goal, &t.outcome).unwrap();
        for s in &t.states {
            let tree = parse_indented(&s.accessibility_tree).unwrap();
            let action = s.action.clone().map(Action::Call);
            x.ingest(&Observation {
                episode: &t.id,
                step: s.state_index,
                tree: &tree,
                url: Some(&s.url),
                action: action.as_ref(),
            })
            .unwrap();
        }
    }
    x
}

fn named(x: &Experience, m: &HashMap<Sym, u32>) -> BTreeMap<String, u32> {
    m.iter()
        .map(|(&s, &c)| (x.terms.syms.get(s).to_string(), c))
        .collect()
}

fn tkey(x: &Experience, id: u32) -> String {
    let i = x.info(id);
    format!("{}||{}", i.url_template, i.title_template)
}

#[test]
fn matches_the_reference_model_on_the_committed_fixture() {
    let f = fixture();
    let x = parity_store();

    let mut got: Vec<UnitKey> = x
        .regions
        .slots
        .iter()
        .flatten()
        .map(|r| {
            let vals = r.values.iter().map(|(k, &v)| (k.clone(), v)).collect();
            (
                r.text.clone(),
                r.breadcrumb.clone(),
                r.occurrences(),
                r.episodes(),
                vals,
            )
        })
        .collect();
    got.sort();
    let want: Vec<_> = f
        .units
        .iter()
        .map(|u| {
            (
                u.text.clone(),
                u.breadcrumb.clone(),
                u.occurrences,
                u.episodes,
                u.values.clone(),
            )
        })
        .collect();
    assert_eq!(got.len(), want.len());
    assert_eq!(
        got, want,
        "same regions, breadcrumbs, occurrence counts and value lines"
    );

    assert_eq!(x.effects.rows.len(), f.effects.len());
    for e in &f.effects {
        let row = x
            .effects
            .rows
            .iter()
            .find(|((t, l), _)| tkey(&x, *t) == e.template && x.terms.syms.get(*l) == e.action)
            .unwrap_or_else(|| panic!("missing effect row {} {}", e.template, e.action))
            .1;
        assert_eq!((row.n, row.loops), (e.n, e.loops), "{}", e.action);
        assert_eq!(named(&x, &row.add), e.add, "{}", e.action);
        assert_eq!(named(&x, &row.rem), e.rem, "{}", e.action);
        assert_eq!(named(&x, &row.err), e.err, "{}", e.action);
        let next: BTreeMap<String, u32> = row.nxt.iter().map(|(&t, &c)| (tkey(&x, t), c)).collect();
        assert_eq!(next, e.next, "{}", e.action);
    }

    assert_eq!(x.templates.live(), f.templates.len());
    for t in &f.templates {
        let id = (0..x.templates.slots.len() as u32)
            .find(|&i| x.templates.get(i).is_some() && tkey(&x, i) == t.template)
            .unwrap_or_else(|| panic!("missing template {}", t.template));
        let rec = x.templates.get(id).unwrap();
        assert_eq!(rec.obs, t.obs);
        assert_eq!(named(&x, &rec.inventory), t.inventory, "{}", t.template);
    }

    for (id, tail) in &f.episodes {
        let text = x.episode_text(x.episode_slot(id).unwrap());
        assert_eq!(
            text.split_once('\n').map(|p| p.1),
            Some(tail.as_str()),
            "{id}"
        );
    }
}

fn props_step(episode: &str, step: u32, props: &str) -> OwnedObservation {
    let mut tree = page("P", vec![btn("Open"), btn("Save")]);
    tree.props = props.to_string();
    OwnedObservation {
        episode: episode.into(),
        step,
        tree,
        url: None,
        action: None,
        goal: None,
        outcome: None,
    }
}

#[test]
fn steps_differing_only_in_props_count_against_the_quota() {
    let dir = tempfile::tempdir().unwrap();
    let mut s =
        store::ExperienceStore::create(dir.path(), ExperienceConfig::default(), None).unwrap();
    s.ingest_many(vec![props_step("e", 0, "p0")]).unwrap();
    let base = s.quota_units();
    let budget = base + 2;
    // limit - 1, limit, then limit + 1.
    for step in 1..=2u32 {
        let r = s
            .ingest_many_within(
                vec![props_step("e", step, &format!("p{step}"))],
                Some(budget),
            )
            .unwrap();
        assert_eq!(r, [Ingested::Applied]);
    }
    assert_eq!(s.quota_units(), budget);
    let (at, err) = s
        .ingest_many_within(vec![props_step("e", 3, "p3")], Some(budget))
        .unwrap_err();
    assert_eq!((at, matches!(err, ExperienceError::Quota(_))), (0, true));
    assert_eq!(s.quota_units(), budget, "a refused step leaves nothing");
    // A retry of a stored step adds nothing and is still accepted at the limit.
    let r = s
        .ingest_many_within(vec![props_step("e", 2, "p2")], Some(budget))
        .unwrap();
    assert_eq!(r, [Ingested::Duplicate]);
}

#[test]
fn a_write_that_would_pass_the_snapshot_limit_is_refused_before_it_is_journaled() {
    let big = "q".repeat(4000);
    let steps = |n: u32| -> Vec<OwnedObservation> {
        (0..n)
            .map(|i| props_step("e", i, &format!("{big}{i}")))
            .collect()
    };
    let n = 6;
    let mut probe = Experience::new(ExperienceConfig::default());
    for o in steps(n) {
        probe.ingest(&o.as_observation()).unwrap();
    }
    let exact = probe.to_bytes_at(u64::MAX).unwrap().len() as u64 - 20;
    for (cap, fits) in [(exact + 1, true), (exact, true), (exact - 1, false)] {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ExperienceConfig {
            max_snapshot_bytes: cap,
            ..ExperienceConfig::default()
        };
        let mut s = store::ExperienceStore::create(dir.path(), cfg.clone(), None).unwrap();
        let r = s.ingest_many(steps(n));
        assert_eq!(r.is_ok(), fits, "cap {cap}");
        let stored = if fits { n } else { n - 1 } as usize;
        if !fits {
            let (at, err) = r.unwrap_err();
            assert_eq!(at, stored);
            assert!(matches!(err, ExperienceError::Limit { .. }), "{err}");
        } else {
            assert!(s
                .ingest_many(vec![props_step("e", n, &format!("{big}{n}"))])
                .is_err());
        }
        // Checkpoint writes a snapshot the reader accepts, and the reopened store has every
        // acknowledged step.
        s.checkpoint().unwrap();
        drop(s);
        let s = store::ExperienceStore::open(dir.path(), cfg, None).unwrap();
        assert_eq!(s.stats().steps, stored, "cap {cap}");
    }
}

#[test]
fn an_append_after_a_zeroed_header_is_damage_not_a_scrubbed_journal() {
    let dir = tempfile::tempdir().unwrap();
    let mut s =
        store::ExperienceStore::create(dir.path(), ExperienceConfig::default(), None).unwrap();
    s.ingest_many(vec![props_step("e", 0, "a")]).unwrap();
    s.checkpoint().unwrap();
    s.ingest_many(vec![props_step("e", 1, "b")]).unwrap();
    drop(s);
    let journal = dir.path().join(store::JOURNAL_FILE);
    let mut bytes = std::fs::read(&journal).unwrap();
    bytes[..8].fill(0);
    std::fs::write(&journal, &bytes).unwrap();
    let err = store::ExperienceStore::open(dir.path(), ExperienceConfig::default(), None)
        .err()
        .expect("acknowledged record after zeros");
    assert!(matches!(err, ExperienceError::Snapshot(_)), "{err}");
    // A journal scrubbed whole, which a finished checkpoint leaves behind, still opens.
    bytes.fill(0);
    std::fs::write(&journal, &bytes).unwrap();
    store::ExperienceStore::open(dir.path(), ExperienceConfig::default(), None).unwrap();
}

#[test]
fn step_controls_round_trip_the_snapshot_and_purge_with_the_episode() {
    let mut x = Experience::default();
    let page_a = page("Form", vec![btn("Save")]);
    let controls = x
        .extract_controls(
            "[1] textbox 'Name', value='Ada'\n[2] combobox 'Size'\n\t[3] option 'S', selected=True",
        )
        .unwrap();
    assert_eq!(controls.len(), 2);
    x.ingest_with_controls(&obs("a", 0, &page_a, None), controls.clone())
        .unwrap();
    x.ingest(&obs("b", 0, &page_a, None)).unwrap();
    let bytes = x.to_bytes().unwrap();
    assert_eq!(bytes[4], 5, "episode ids and handle records need version 5");
    let y = Experience::from_bytes(&bytes, ExperienceConfig::default()).unwrap();
    assert_eq!(y.step_controls("a", 0), Some(&controls[..]));
    assert_eq!(y.step_controls("b", 0), Some(&[][..]));
    assert_eq!(y.step_controls("a", 1), None);
    assert_eq!(y.to_bytes().unwrap(), bytes);
    assert_eq!(
        y.form_tables(&[("a", 0), ("b", 0)]),
        x.form_tables(&[("a", 0), ("b", 0)])
    );

    // A version 2 header over a payload with controls is refused, not read without them.
    let mut old = bytes.clone();
    old[4] = 2;
    assert!(Experience::from_bytes(&old, ExperienceConfig::default()).is_err());

    // Episode ids need version 5 once any episode has been stored, with or without controls.
    x.purge_episode("a").unwrap();
    assert_eq!(x.to_bytes().unwrap()[4], 5);
}

#[test]
fn caller_controls_are_bounded_and_the_switch_drops_the_page_text() {
    let mut x = Experience::default();
    let t = page("Form", vec![btn("Save")]);
    let one = Control {
        role: "checkbox".into(),
        name: "c".into(),
        ..Control::default()
    };
    let too_many = vec![one.clone(); form::MAX_CONTROLS + 1];
    assert!(matches!(
        x.ingest_with_controls(&obs("a", 0, &t, None), too_many),
        Err(ExperienceError::Limit { .. })
    ));
    let mut wide = one.clone();
    wide.options = vec![
        ControlOption {
            label: "o".into(),
            selected: false,
        };
        form::MAX_OPTIONS + 1
    ];
    assert!(x
        .ingest_with_controls(&obs("a", 0, &t, None), vec![wide])
        .is_err());
    let mut long = one;
    long.name = "n".repeat(x.config().max_text_bytes + 1);
    assert!(x
        .ingest_with_controls(&obs("a", 0, &t, None), vec![long])
        .is_err());
    assert!(!x.has_episode("a"));

    let off = Experience::new(ExperienceConfig {
        form_state: false,
        ..Default::default()
    });
    assert!(off.extract_controls("[1] textbox 'x'").unwrap().is_empty());
}

/// `n` textbox controls whose text totals `total` bytes.
fn boxes_totalling(n: usize, total: usize) -> Vec<Control> {
    let names = total - n * "textbox".len();
    (0..n)
        .map(|i| Control {
            role: "textbox".into(),
            name: "n".repeat(names / n + usize::from(i < names % n)),
            ..Control::default()
        })
        .collect()
}

#[test]
fn caller_controls_are_bounded_by_the_step_budget_and_charged_to_the_quota() {
    let t = page("Form", vec![btn("Save")]);
    for (step, total, ok) in [
        (0, form::MAX_CONTROL_BYTES - 1, true),
        (1, form::MAX_CONTROL_BYTES, true),
        (2, form::MAX_CONTROL_BYTES + 1, false),
    ] {
        let mut x = Experience::default();
        let mut plain = Experience::default();
        plain.ingest(&obs("a", step, &t, None)).unwrap();
        let controls = boxes_totalling(65, total);
        assert_eq!(form::text_bytes(&controls), total);
        let r = x.ingest_with_controls(&obs("a", step, &t, None), controls);
        if ok {
            r.unwrap();
            assert_eq!(
                x.quota_units() - plain.quota_units(),
                total.div_ceil(form::QUOTA_UNIT_BYTES) as u64
            );
        } else {
            assert!(matches!(r, Err(ExperienceError::Limit { .. })), "{r:?}");
            assert!(!x.has_episode("a"));
        }
    }
}

#[test]
fn the_snapshot_estimate_bounds_the_encoded_controls() {
    let full = |s: &str| Control {
        role: s.into(),
        name: s.into(),
        value: Some(s.into()),
        checked: Some(s.into()),
        selected: Some(s.into()),
        expanded: Some(s.into()),
        pressed: Some(s.into()),
        required: Some(true),
        disabled: Some(true),
        readonly: Some(true),
        options: vec![
            ControlOption {
                label: s.into(),
                selected: true,
            };
            form::MAX_OPTIONS
        ],
        more_options: u32::MAX,
    };
    let t = page("Form", vec![btn("Save")]);
    let mut x = Experience::default();
    let mut plain = Experience::default();
    for step in 0..8 {
        x.ingest_with_controls(
            &obs("a", step, &t, None),
            vec![full(""); form::MAX_CONTROLS],
        )
        .unwrap();
        plain.ingest(&obs("a", step, &t, None)).unwrap();
    }
    let added = x.to_bytes().unwrap().len() - plain.to_bytes().unwrap().len();
    let estimated = x.snapshot_estimate() - plain.snapshot_estimate();
    assert!(estimated >= added as u64, "{estimated} < {added}");
    assert!(x.snapshot_estimate() >= x.to_bytes().unwrap().len() as u64);
}

/// Rewrites every `controls` list of a snapshot payload and reseals the header.
fn mutate_controls(bytes: &[u8], f: &dyn Fn(&mut Vec<ciborium::Value>)) -> Vec<u8> {
    fn walk(v: &mut ciborium::Value, f: &dyn Fn(&mut Vec<ciborium::Value>)) {
        match v {
            ciborium::Value::Map(m) => {
                for (k, v) in m.iter_mut() {
                    match (k.as_text(), v) {
                        (Some("controls"), ciborium::Value::Array(a)) => f(a),
                        (_, v) => walk(v, f),
                    }
                }
            }
            ciborium::Value::Array(a) => a.iter_mut().for_each(|v| walk(v, f)),
            _ => {}
        }
    }
    let mut v: ciborium::Value = ciborium::from_reader(&bytes[20..]).unwrap();
    walk(&mut v, f);
    let mut payload = Vec::new();
    ciborium::into_writer(&v, &mut payload).unwrap();
    let mut out = bytes[..20].to_vec();
    out[8..16].copy_from_slice(&(payload.len() as u64).to_le_bytes());
    out[16..20].copy_from_slice(&crc32fast::hash(&payload).to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

#[test]
fn a_version_3_snapshot_over_the_control_caps_is_refused() {
    let mut x = Experience::default();
    let t = page("Form", vec![btn("Save")]);
    let listbox = Control {
        role: "listbox".into(),
        name: "l".into(),
        options: vec![
            ControlOption {
                label: "o".into(),
                selected: false,
            };
            form::MAX_OPTIONS
        ],
        ..Control::default()
    };
    x.ingest_with_controls(&obs("a", 0, &t, None), vec![listbox; form::MAX_CONTROLS])
        .unwrap();
    let bytes = x.to_bytes().unwrap();
    assert_eq!(bytes[4], 5, "episode ids and handle records need version 5");
    let same = mutate_controls(&bytes, &|_| {});
    Experience::from_bytes(&same, ExperienceConfig::default()).unwrap();

    let more_controls = mutate_controls(&bytes, &|a| a.push(a[0].clone()));
    assert!(Experience::from_bytes(&more_controls, ExperienceConfig::default()).is_err());
    let more_options = mutate_controls(&bytes, &|a| {
        if let ciborium::Value::Map(m) = &mut a[0] {
            for (k, v) in m.iter_mut() {
                if let (Some("options"), ciborium::Value::Array(o)) = (k.as_text(), v) {
                    o.push(o[0].clone());
                }
            }
        }
    });
    assert!(Experience::from_bytes(&more_options, ExperienceConfig::default()).is_err());
}

#[test]
fn a_property_only_change_is_no_listed_change_but_a_state_change() {
    let mut x = Experience::default();
    let off = page("P", vec![btn("Agree"), btn("Save")]);
    let mut on = off.clone();
    on.children[0].props = "checked".into();
    run(&mut x, "e", &[off.clone(), on, off.clone(), off]);
    let path = x.episode("e").unwrap().path;
    assert!(
        path[1].no_change && path[1].state_changed,
        "checkbox toggled"
    );
    assert!(path[2].no_change && path[2].state_changed, "toggled back");
    assert!(
        path[3].no_change && !path[3].state_changed,
        "nothing changed"
    );
}

mod occurrence_targets {
    use super::*;
    use crate::core::experience::{
        ActionRecord, OccurrenceNode, OccurrenceRef, Resolution, TargetRole, Unresolved,
    };

    fn h(mut n: TreeNode, handle: &str) -> TreeNode {
        n.handle = Some(handle.into());
        n
    }

    /// A table of rows, each with a same-named Delete button carrying its own handle.
    fn rows(handles: [&str; 2]) -> TreeNode {
        let row = |who: &str, hd: &str| {
            TreeNode::new("row", who).with_children(vec![h(TreeNode::new("button", "Delete"), hd)])
        };
        TreeNode::new("RootWebArea", "Tickets")
            .with_children(vec![row("INC1", handles[0]), row("INC2", handles[1])])
    }

    fn call(c: &str) -> Action {
        Action::Call(c.into())
    }

    fn put(x: &mut Experience, step: u32, t: &TreeNode, a: Option<&Action>) -> Ingested {
        x.ingest(&Observation {
            episode: "e",
            step,
            tree: t,
            url: Some("http://h/t"),
            action: a,
        })
        .unwrap()
    }

    fn target_of(x: &Experience, step: u32) -> (ActionRecord, Option<OccurrenceRef>) {
        let t = x.action_targets("e", step).unwrap();
        (t.record, t.targets.into_iter().next().and_then(|(_, r)| r))
    }

    #[test]
    fn same_named_controls_in_different_rows_get_different_addresses() {
        let mut x = Experience::default();
        let t0 = rows(["7", "9"]);
        let gone = TreeNode::new("RootWebArea", "Tickets");
        put(&mut x, 0, &t0, None);
        put(&mut x, 1, &gone, Some(&call("click('7')")));
        put(&mut x, 2, &t0, None);
        put(&mut x, 3, &gone, Some(&call("click('9')")));
        let (r1, a) = target_of(&x, 1);
        let (r3, b) = target_of(&x, 3);
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_ne!(a.path, b.path);
        assert_eq!((a.step, b.step), (0, 2));
        assert_eq!(x.occurrence(&a).unwrap().name, "Delete");
        assert_eq!(a.path, vec![0, 0]);
        assert_eq!(b.path, vec![1, 0]);
        // The display labels and successor trees are identical; the authoritative records are not.
        let label = |s: usize| x.episode("e").unwrap().path[s].action.clone();
        assert_eq!(label(1), label(3));
        assert_ne!(r1, r3);
    }

    #[test]
    fn identical_sibling_subtrees_stay_separately_addressable() {
        let mut x = Experience::default();
        let a = h(TreeNode::new("button", "Go"), "1");
        let b = h(TreeNode::new("button", "Go"), "2");
        let t = TreeNode::new("RootWebArea", "P").with_children(vec![a, b]);
        let mut terms = TermStore::default();
        let ids = (terms.intern(&t.children[0]), terms.intern(&t.children[1]));
        assert_eq!(ids.0, ids.1, "the contents are one shared node");
        put(&mut x, 0, &t, None);
        put(&mut x, 1, &t, Some(&call("click('2')")));
        let (_, r) = target_of(&x, 1);
        assert_eq!(r.unwrap().path, vec![1]);
    }

    #[test]
    fn a_reused_handle_resolves_against_its_own_observation() {
        let mut x = Experience::default();
        let first = rows(["7", "9"]);
        let second = rows(["9", "7"]);
        put(&mut x, 0, &first, None);
        put(&mut x, 1, &second, Some(&call("click('7')")));
        put(&mut x, 2, &first, Some(&call("click('7')")));
        let (_, r1) = target_of(&x, 1);
        let (_, r2) = target_of(&x, 2);
        assert_eq!(
            r1.unwrap().path,
            vec![0, 0],
            "handle 7 was row 1 when first clicked"
        );
        assert_eq!(
            r2.unwrap().path,
            vec![1, 0],
            "and row 2 in the next observation"
        );
    }

    #[test]
    fn unresolvable_targets_stay_unresolved_with_their_reason() {
        let mut x = Experience::default();
        let first = put(&mut x, 0, &rows(["7", "9"]), Some(&call("click('7')")));
        assert_eq!(first, Ingested::Applied);
        let why = |x: &Experience, s| match x.action_targets("e", s).unwrap().record {
            ActionRecord::Call { targets, .. } => targets[0].resolution.clone(),
            r => panic!("{r:?}"),
        };
        assert_eq!(
            why(&x, 0),
            Resolution::Unresolved(Unresolved::NoPredecessor)
        );
        put(&mut x, 1, &rows(["7", "9"]), Some(&call("click('99')")));
        assert_eq!(
            why(&x, 1),
            Resolution::Unresolved(Unresolved::HandleNotFound)
        );
        let dup = TreeNode::new("RootWebArea", "T").with_children(vec![
            h(TreeNode::new("button", "A"), "5"),
            h(TreeNode::new("button", "B"), "5"),
        ]);
        put(&mut x, 2, &dup, Some(&call("click('7')")));
        put(&mut x, 3, &dup, Some(&call("click('5')")));
        assert_eq!(
            why(&x, 3),
            Resolution::Unresolved(Unresolved::HandleConflict)
        );
        // A gap in the capture: step 5 does not follow step 3.
        put(&mut x, 5, &dup, Some(&call("click('5')")));
        assert_eq!(
            why(&x, 5),
            Resolution::Unresolved(Unresolved::Discontinuity)
        );
        assert!(x.action_targets("e", 5).unwrap().predecessor.is_none());
        // A caller-supplied label has nothing to resolve and fabricates no address.
        put(
            &mut x,
            6,
            &dup,
            Some(&Action::Label("click button 'A'".into())),
        );
        let t = x.action_targets("e", 6).unwrap();
        assert_eq!(t.record, ActionRecord::Label("click button 'A'".into()));
        assert!(t.targets.is_empty() && t.predecessor.is_none());
    }

    #[test]
    fn different_original_targets_are_not_the_same_ingest() {
        let mut x = Experience::default();
        let t0 = rows(["7", "9"]);
        let gone = TreeNode::new("RootWebArea", "Tickets");
        put(&mut x, 0, &t0, None);
        assert_eq!(
            put(&mut x, 1, &gone, Some(&call("click('7')"))),
            Ingested::Applied
        );
        assert_eq!(
            put(&mut x, 1, &gone, Some(&call("click('7')"))),
            Ingested::Duplicate
        );
        let err = x
            .ingest(&Observation {
                episode: "e",
                step: 1,
                tree: &gone,
                url: Some("http://h/t"),
                action: Some(&call("click('9')")),
            })
            .unwrap_err();
        assert!(matches!(err, ExperienceError::Conflict { .. }), "{err:?}");
    }

    #[test]
    fn drag_and_drop_keeps_each_targets_role() {
        let mut x = Experience::default();
        let t0 = rows(["7", "9"]);
        put(&mut x, 0, &t0, None);
        put(&mut x, 1, &t0, Some(&call("drag_and_drop('9', '7')")));
        let t = x.action_targets("e", 1).unwrap();
        let by: Vec<_> = t
            .targets
            .iter()
            .map(|(r, o)| (*r, o.as_ref().unwrap().path.clone()))
            .collect();
        assert_eq!(
            by,
            vec![
                (TargetRole::Subject, vec![1, 0]),
                (TargetRole::Destination, vec![0, 0])
            ]
        );
    }

    fn chain(x: &mut Experience) {
        let t0 = rows(["7", "9"]);
        put(x, 0, &t0, None);
        put(x, 1, &t0, Some(&call("click('9')")));
    }

    #[test]
    fn the_first_action_after_a_snapshot_resolves_against_the_stored_predecessor() {
        let mut x = Experience::default();
        chain(&mut x);
        let bytes = x.to_bytes().unwrap();
        let mut y = Experience::from_bytes(&bytes, ExperienceConfig::default()).unwrap();
        assert_eq!(
            y.action_targets("e", 1).unwrap().record,
            x.action_targets("e", 1).unwrap().record
        );
        // The handle map of step 1 survived: handle 7 is row 1 of that observation.
        put(&mut y, 2, &rows(["7", "9"]), Some(&call("click('7')")));
        let (_, r) = target_of(&y, 2);
        let r = r.unwrap();
        assert_eq!((r.step, r.path.clone()), (1, vec![0, 0]));
        assert_eq!(y.episode("e").unwrap().uid, x.episode("e").unwrap().uid);
    }

    #[test]
    fn a_store_reopens_from_journal_and_checkpoint_with_the_same_targets() {
        let dir = tempfile::tempdir().unwrap();
        let obs = |step: u32, a: Option<Action>| OwnedObservation {
            episode: "e".into(),
            step,
            tree: rows(["7", "9"]),
            url: Some("http://h/t".into()),
            action: a,
            goal: None,
            outcome: None,
        };
        let mut s =
            store::ExperienceStore::create(dir.path(), ExperienceConfig::default(), None).unwrap();
        s.ingest_many(vec![obs(0, None), obs(1, Some(call("click('9')")))])
            .unwrap();
        let want = s.action_targets("e", 1).unwrap();
        drop(s);
        let mut s =
            store::ExperienceStore::open(dir.path(), ExperienceConfig::default(), None).unwrap();
        assert_eq!(s.action_targets("e", 1).unwrap(), want);
        s.checkpoint().unwrap();
        drop(s);
        // Snapshot, then a journaled action that depends on the checkpointed handle map.
        let mut s =
            store::ExperienceStore::open(dir.path(), ExperienceConfig::default(), None).unwrap();
        assert_eq!(s.action_targets("e", 1).unwrap(), want);
        s.ingest_many(vec![obs(2, Some(call("click('7')")))])
            .unwrap();
        let after = s.action_targets("e", 2).unwrap();
        drop(s);
        let s =
            store::ExperienceStore::open(dir.path(), ExperienceConfig::default(), None).unwrap();
        assert_eq!(s.action_targets("e", 2).unwrap(), after);
        assert_eq!(after.targets[0].1.as_ref().unwrap().path, vec![0, 0]);
        assert_eq!(after.predecessor.as_ref().unwrap().step, 1);
    }

    #[test]
    fn a_purged_episode_is_not_aliased_by_its_replacement() {
        let mut x = Experience::default();
        chain(&mut x);
        let old = target_of(&x, 1).1.unwrap();
        let uid = x.episode("e").unwrap().uid;
        x.purge_episode("e").unwrap();
        assert!(x.occurrence(&old).is_none(), "nothing of the purge remains");
        // Same names, a different tree where the same path addresses something else.
        let other = TreeNode::new("RootWebArea", "Other").with_children(vec![
            TreeNode::new("group", "g").with_children(vec![h(TreeNode::new("link", "L"), "7")]),
            TreeNode::new("group", "g2").with_children(vec![TreeNode::new("link", "M")]),
        ]);
        put(&mut x, 0, &other, None);
        put(&mut x, 1, &other, Some(&call("click('7')")));
        assert_ne!(x.episode("e").unwrap().uid, uid);
        assert!(
            x.occurrence(&old).is_none(),
            "the old reference does not resolve into new data"
        );
        let fresh = target_of(&x, 1).1.unwrap();
        assert_eq!(x.occurrence(&fresh).unwrap().name, "L");
        // The uid is not reused after a reopen either.
        let y =
            Experience::from_bytes(&x.to_bytes().unwrap(), ExperienceConfig::default()).unwrap();
        assert!(y.episode("e").unwrap().uid > uid);
    }

    #[test]
    fn a_snapshot_without_target_records_reads_as_unknown_not_resolved() {
        let mut x = Experience::default();
        chain(&mut x);
        let legacy = super::snapshot::downgrade_to_version_3_for_test(&x.to_bytes().unwrap());
        let mut y = Experience::from_bytes(&legacy, ExperienceConfig::default()).unwrap();
        assert_eq!(
            y.action_targets("e", 1).unwrap().record,
            ActionRecord::Legacy
        );
        assert!(y.action_targets("e", 1).unwrap().targets.is_empty());
        put(&mut y, 2, &rows(["7", "9"]), Some(&call("click('7')")));
        match y.action_targets("e", 2).unwrap().record {
            ActionRecord::Call { targets, .. } => assert_eq!(
                targets[0].resolution,
                Resolution::Unresolved(Unresolved::NoHandleContext)
            ),
            r => panic!("{r:?}"),
        }
        assert!(y.action_targets("e", 2).unwrap().targets[0].1.is_none());
    }

    #[test]
    fn a_fresh_empty_experience_round_trips_and_checkpoints() {
        let x = Experience::default();
        let y =
            Experience::from_bytes(&x.to_bytes().unwrap(), ExperienceConfig::default()).unwrap();
        assert!(y.episode_ids().is_empty());
        // The first episode of the reloaded store still gets uid 1 and nothing is reused.
        let mut y = y;
        put(&mut y, 0, &rows(["7", "9"]), None);
        assert_eq!(y.episode("e").unwrap().uid, 1);

        let dir = tempfile::tempdir().unwrap();
        let mut s =
            store::ExperienceStore::create(dir.path(), ExperienceConfig::default(), None).unwrap();
        s.checkpoint().unwrap();
        drop(s);
        let s =
            store::ExperienceStore::open(dir.path(), ExperienceConfig::default(), None).unwrap();
        assert!(s.episode_ids().is_empty());
    }

    fn owned(episode: &str, step: u32, t: TreeNode, a: Option<Action>) -> OwnedObservation {
        OwnedObservation {
            episode: episode.into(),
            step,
            tree: t,
            url: Some("http://h/t".into()),
            action: a,
            goal: None,
            outcome: None,
        }
    }

    #[test]
    fn a_rejected_description_leaves_no_episode_and_durable_references_survive_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ExperienceConfig::default();
        let mut s = store::ExperienceStore::create(dir.path(), cfg.clone(), None).unwrap();
        let mut bad = owned("bad", 0, rows(["7", "9"]), None);
        bad.goal = Some("g".repeat(cfg.max_goal_bytes + 1));
        let (at, err) = s.ingest_many(vec![bad.clone()]).unwrap_err();
        assert_eq!(at, 0, "{err:?}");
        let mut bad_outcome = bad.clone();
        bad_outcome.goal = None;
        bad_outcome.outcome = Some("o".repeat(cfg.max_goal_bytes + 1));
        assert!(s.ingest_many(vec![bad_outcome]).is_err());
        assert!(!s.has_episode("bad"), "a rejected operation stores nothing");

        // An earlier success in a batch is kept when a later operation is rejected.
        let mut later_bad = owned("late", 0, rows(["7", "9"]), None);
        later_bad.goal = Some("g".repeat(cfg.max_goal_bytes + 1));
        let (at, _) = s
            .ingest_many(vec![
                owned("a", 0, rows(["7", "9"]), None),
                owned("a", 1, rows(["7", "9"]), Some(call("click('9')"))),
                later_bad,
            ])
            .unwrap_err();
        assert_eq!(at, 2);
        assert!(s.has_episode("a") && !s.has_episode("late"));
        s.ingest_many(vec![
            owned("b", 0, rows(["5", "6"]), None),
            owned("b", 1, rows(["5", "6"]), Some(call("click('5')"))),
        ])
        .unwrap();

        let held: Vec<(String, u64, OccurrenceRef, OccurrenceNode)> = ["a", "b"]
            .iter()
            .map(|e| {
                let t = s.action_targets(e, 1).unwrap();
                let r = t.targets[0].1.clone().unwrap();
                let n = s.occurrence(&r).unwrap();
                (e.to_string(), s.episode(e).unwrap().uid, r, n)
            })
            .collect();
        assert_ne!(held[0].1, held[1].1);
        drop(s);
        let s = store::ExperienceStore::open(dir.path(), cfg, None).unwrap();
        for (e, uid, r, n) in &held {
            assert_eq!(
                s.episode(e).unwrap().uid,
                *uid,
                "uid of {e} changed on reopen"
            );
            assert_eq!(
                s.occurrence(r).as_ref(),
                Some(n),
                "reference into {e} moved"
            );
        }
    }

    fn chain3(x: &mut Experience) {
        put(x, 0, &rows(["7", "9"]), None);
        put(x, 1, &rows(["7", "9"]), Some(&call("click('7')")));
        put(x, 2, &rows(["7", "9"]), Some(&call("click('9')")));
    }

    fn retry(x: &mut Experience, step: u32, t: &TreeNode, a: Option<&Action>) -> Result<Ingested> {
        x.ingest(&Observation {
            episode: "e",
            step,
            tree: t,
            url: Some("http://h/t"),
            action: a,
        })
    }

    #[test]
    fn a_retry_with_a_changed_handle_map_is_a_conflict_not_a_duplicate() {
        let mut x = Experience::default();
        chain3(&mut x);
        let a1 = call("click('7')");
        // Same handles: a duplicate, for the latest step and for an older one.
        assert_eq!(
            retry(&mut x, 1, &rows(["7", "9"]), Some(&a1)).unwrap(),
            Ingested::Duplicate
        );
        assert_eq!(
            retry(&mut x, 0, &rows(["7", "9"]), None).unwrap(),
            Ingested::Duplicate
        );
        // Swapped handles: the content is the same, the addresses are not.
        let c = retry(&mut x, 1, &rows(["9", "7"]), Some(&a1));
        assert!(matches!(c, Err(ExperienceError::Conflict { .. })), "{c:?}");
        let c = retry(&mut x, 0, &rows(["9", "7"]), None);
        assert!(
            matches!(c, Err(ExperienceError::Conflict { .. })),
            "older step: {c:?}"
        );
        // A handle removed, and a handle added.
        let mut removed = rows(["7", "9"]);
        removed.children[1].children[0].handle = None;
        assert!(matches!(
            retry(&mut x, 0, &removed, None),
            Err(ExperienceError::Conflict { .. })
        ));
        let mut added = rows(["7", "9"]);
        added.children[0].handle = Some("1".into());
        assert!(matches!(
            retry(&mut x, 0, &added, None),
            Err(ExperienceError::Conflict { .. })
        ));
        // Two elements with one handle.
        assert!(matches!(
            retry(&mut x, 0, &rows(["7", "7"]), None),
            Err(ExperienceError::Conflict { .. })
        ));
        // Nothing the rejected retries carried changed the stored targets.
        let t = x.action_targets("e", 2).unwrap().targets[0]
            .1
            .clone()
            .unwrap();
        assert_eq!(t.path, vec![1, 0]);
    }

    #[test]
    fn a_legacy_step_without_handle_context_keeps_its_targets_unknown() {
        let mut x = Experience::default();
        chain3(&mut x);
        let legacy = super::snapshot::downgrade_to_version_3_for_test(&x.to_bytes().unwrap());
        let mut y = Experience::from_bytes(&legacy, ExperienceConfig::default()).unwrap();
        // The retry cannot be compared with handles that were never recorded, so it is the
        // duplicate it was before version 4; it does not upgrade the step to a resolved target.
        let a = call("click('7')");
        assert_eq!(
            retry(&mut y, 1, &rows(["9", "7"]), Some(&a)).unwrap(),
            Ingested::Duplicate
        );
        assert_eq!(
            y.action_targets("e", 1).unwrap().record,
            ActionRecord::Legacy
        );
        assert!(y.action_targets("e", 1).unwrap().targets.is_empty());
    }

    #[test]
    fn the_handle_digest_survives_a_snapshot() {
        let mut x = Experience::default();
        chain3(&mut x);
        let mut y =
            Experience::from_bytes(&x.to_bytes().unwrap(), ExperienceConfig::default()).unwrap();
        let a = call("click('7')");
        assert!(matches!(
            retry(&mut y, 1, &rows(["9", "7"]), Some(&a)),
            Err(ExperienceError::Conflict { .. })
        ));
        assert_eq!(
            retry(&mut y, 1, &rows(["7", "9"]), Some(&a)).unwrap(),
            Ingested::Duplicate
        );
    }

    #[test]
    fn a_version_4_snapshot_without_digests_refuses_retries_it_cannot_check() {
        let mut x = Experience::default();
        chain3(&mut x);
        let v4 = super::snapshot::strip_digests_to_version_4_for_test(&x.to_bytes().unwrap());
        assert_eq!(v4[4], 4);
        let mut y = Experience::from_bytes(&v4, ExperienceConfig::default()).unwrap();
        let (a1, a2) = (call("click('7')"), call("click('9')"));
        // The older steps' handle addresses are gone: no retry of them is a duplicate, and a
        // changed one is still refused, never accepted as the same observation.
        for t in [rows(["7", "9"]), rows(["9", "7"])] {
            let r = retry(&mut y, 1, &t, Some(&a1));
            assert!(
                matches!(r, Err(ExperienceError::Unverifiable { step: 1, .. })),
                "{r:?}"
            );
        }
        // Different content is a conflict whatever the handles.
        assert!(matches!(
            retry(&mut y, 1, &rows(["7", "9"]), Some(&a2)),
            Err(ExperienceError::Conflict { .. })
        ));
        // The latest step's addresses are the episode's stored handle map, so it is checked.
        assert_eq!(
            retry(&mut y, 2, &rows(["7", "9"]), Some(&a2)).unwrap(),
            Ingested::Duplicate
        );
        assert!(matches!(
            retry(&mut y, 2, &rows(["9", "7"]), Some(&a2)),
            Err(ExperienceError::Conflict { .. })
        ));
        // The targets resolved before the upgrade are kept, unlike a legacy step's.
        assert_eq!(
            y.action_targets("e", 1).unwrap().targets[0]
                .1
                .as_ref()
                .unwrap()
                .path,
            vec![0, 0]
        );
        // A rewrite keeps the distinction instead of turning the step into a legacy one.
        let v5 = y.to_bytes().unwrap();
        assert_eq!(v5[4], 5);
        let mut z = Experience::from_bytes(&v5, ExperienceConfig::default()).unwrap();
        assert!(matches!(
            retry(&mut z, 1, &rows(["7", "9"]), Some(&a1)),
            Err(ExperienceError::Unverifiable { .. })
        ));
        assert_eq!(
            retry(&mut z, 2, &rows(["7", "9"]), Some(&a2)).unwrap(),
            Ingested::Duplicate
        );
        put(&mut z, 3, &rows(["7", "9"]), Some(&a1));
        assert!(matches!(
            retry(&mut z, 3, &rows(["9", "7"]), Some(&a1)),
            Err(ExperienceError::Conflict { .. })
        ));
    }

    #[test]
    fn a_step_refused_over_quota_after_it_applied_consumes_no_uid() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ExperienceConfig::default();
        let mut s = store::ExperienceStore::create(dir.path(), cfg.clone(), None).unwrap();
        s.ingest_many(vec![owned("a", 0, rows(["7", "9"]), None)])
            .unwrap();
        // A new episode adds an episode and a step: one unit of headroom is not enough, and the
        // step is refused after it applied in memory.
        let budget = s.quota_units() + 1;
        let (_, err) = s
            .ingest_many_within(vec![owned("over", 0, rows(["1", "2"]), None)], Some(budget))
            .unwrap_err();
        assert!(matches!(err, ExperienceError::Quota(_)), "{err:?}");
        assert!(!s.has_episode("over"));
        s.ingest_many(vec![
            owned("b", 0, rows(["5", "6"]), None),
            owned("b", 1, rows(["5", "6"]), Some(call("click('6')"))),
        ])
        .unwrap();
        let uid = s.episode("b").unwrap().uid;
        let r = s.action_targets("b", 1).unwrap().targets[0]
            .1
            .clone()
            .unwrap();
        drop(s);
        let s = store::ExperienceStore::open(dir.path(), cfg, None).unwrap();
        assert_eq!(s.episode("b").unwrap().uid, uid);
        assert_eq!(s.occurrence(&r).unwrap().name, "Delete");
        assert_eq!(r.path, vec![1, 0]);
    }
}
