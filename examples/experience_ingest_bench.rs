// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Ingest throughput and stored bytes per step of the experience core on synthetic
//! agent trajectories, against the raw trees as indented text.
//!
//! `cargo run --release --features experience --example experience_ingest_bench -- OUT.json`
//!
//! The synthetic site has 12 page types that share a 30-link navigation menu; a page body is a
//! table whose rows name records drawn from a pool of 50. An episode is a walk of 30 steps over
//! the page types. The measured range is 10, 100 and 1000 episodes; nothing is extrapolated.

use std::time::Instant;

use serde_json::json;
use writerslogic_warm::core::experience::{Action, Experience, Observation, TreeNode};

const PAGE_TYPES: u64 = 12;
const RECORDS: u64 = 50;
const STEPS: u32 = 30;
const ROWS: u64 = 25;
const NAV: u64 = 30;

struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

fn node(role: &str, name: &str, children: Vec<TreeNode>) -> TreeNode {
    TreeNode::new(role, name).with_children(children)
}

fn page(kind: u64, record: u64) -> TreeNode {
    let nav = node(
        "navigation",
        "Main menu",
        (0..NAV)
            .map(|i| {
                node(
                    "link",
                    &format!("Section {i} of the portal"),
                    vec![node(
                        "StaticText",
                        &format!("Section {i} of the portal"),
                        vec![],
                    )],
                )
            })
            .collect(),
    );
    let rows = (0..ROWS)
        .map(|r| {
            let id = (record + r * 7 + kind) % RECORDS;
            node(
                "row",
                "",
                vec![node(
                    "cell",
                    "",
                    vec![node(
                        "link",
                        &format!("Record {id} of kind {kind}"),
                        vec![node(
                            "StaticText",
                            &format!("Record {id} of kind {kind}"),
                            vec![],
                        )],
                    )],
                )],
            )
        })
        .collect();
    node(
        "RootWebArea",
        &format!("Kind {kind} page for record {record}"),
        vec![
            nav,
            node(
                "main",
                "",
                vec![node("table", &format!("Records of kind {kind}"), rows)],
            ),
        ],
    )
}

fn raw_bytes(t: &TreeNode, depth: usize) -> usize {
    let own = depth + t.role.len() + t.name.len() + t.props.len() + 6;
    own + t
        .children
        .iter()
        .map(|c| raw_bytes(c, depth + 1))
        .sum::<usize>()
}

fn count(t: &TreeNode) -> usize {
    1 + t.children.iter().map(count).sum::<usize>()
}

fn main() {
    let out = std::env::args().nth(1);
    let mut runs = Vec::new();
    for episodes in [10u64, 100, 1000] {
        let mut rng = Lcg(42);
        let mut walk = Vec::new();
        for e in 0..episodes {
            for s in 0..STEPS {
                walk.push((e, s, rng.next(PAGE_TYPES), rng.next(RECORDS)));
            }
        }
        let trees: Vec<TreeNode> = walk.iter().map(|w| page(w.2, w.3)).collect();
        let nodes: usize = trees.iter().map(count).sum();
        let raw: usize = trees.iter().map(|t| raw_bytes(t, 0)).sum();
        let mut x = Experience::default();
        let started = Instant::now();
        for (w, t) in walk.iter().zip(&trees) {
            let action = Action::Label(format!("click link 'Record {} of kind {}'", w.3, w.2));
            let id = format!("episode-{}", w.0);
            x.ingest(&Observation {
                episode: &id,
                step: w.1,
                tree: t,
                url: Some(&format!("http://site/kind{}/record/{}", w.2, w.3)),
                action: (w.1 > 0).then_some(&action),
            })
            .expect("synthetic trees are within limits");
        }
        let secs = started.elapsed().as_secs_f64();
        let stored = x.to_bytes().expect("snapshot").len();
        let stats = x.stats();
        let started = Instant::now();
        let queries = 200;
        for q in 0..queries {
            let hits = x
                .search_regions(
                    &format!("Record {} of kind {}", q % RECORDS, q % PAGE_TYPES),
                    10,
                )
                .expect("search");
            assert!(!hits.is_empty());
        }
        let query_ms = started.elapsed().as_secs_f64() * 1000.0 / queries as f64;
        let steps = walk.len();
        runs.push(json!({
            "episodes": episodes,
            "steps": steps,
            "raw_nodes": nodes,
            "raw_text_bytes": raw,
            "stored_nodes": stats.nodes,
            "regions": stats.regions,
            "region_occurrences": stats.region_occurrences,
            "snapshot_bytes": stored,
            "snapshot_bytes_per_step": stored as f64 / steps as f64,
            "raw_text_bytes_per_step": raw as f64 / steps as f64,
            "snapshot_to_raw_ratio": stored as f64 / raw as f64,
            "ingest_seconds": secs,
            "ingest_steps_per_second": steps as f64 / secs,
            "ingest_nodes_per_second": nodes as f64 / secs,
            "region_query_ms_mean": query_ms,
        }));
        eprintln!(
            "{episodes} episodes: {:.0} steps/s, ratio {:.4}",
            steps as f64 / secs,
            stored as f64 / raw as f64
        );
    }
    let report = json!({
        "benchmark": "experience_ingest_synthetic",
        "synthetic": {"page_types": PAGE_TYPES, "records": RECORDS, "steps_per_episode": STEPS, "table_rows": ROWS, "nav_links": NAV},
        "tested_range": "10, 100 and 1000 episodes of 30 steps; 12 page types; one machine",
        "runs": runs,
    });
    let text = serde_json::to_string_pretty(&report).expect("json");
    match out {
        Some(p) => std::fs::write(p, text + "\n").expect("write result"),
        None => println!("{text}"),
    }
}
