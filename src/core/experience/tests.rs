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
    let mut v2 = bytes.clone();
    v2[4] = 2;
    assert!(Experience::from_bytes(&v2, ExperienceConfig::default()).is_err());
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
