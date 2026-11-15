# Experience core

The experience core is memory for observations that are trees and that repeat: the accessibility
tree of a web page an agent looks at after every action, a UI hierarchy, a directory listing. It
is off by default and sits behind the `experience` cargo feature. Search over documents is not
changed by it.

## What it stores

One call ingests one observation step: a tree, the action that led to it, an episode id and a
step index.

| Part | What it holds |
|---|---|
| Term store | Every node of every tree, hash-consed: a node with the same role, name, state and children is stored once, whichever episode or page it came from. Each node has a content id (state included) and a structural id (state excluded). Every holder is reference-counted. |
| Region store | Each tree is cut along subtree boundaries into regions of at most 1500 rendered characters. A region is a run of sibling subtrees, identified by their structural ids, with a breadcrumb of the three nearest ancestors and the list of (episode, step) where it occurs. The regions are indexed by the same stemmed BM25 as the document store (`MemoryDocumentStore`); there is no second ranker. |
| Effect table | For each (page template, action label): how often it ran, how often nothing changed, the lines it added and removed, the pages it led to and any error-like lines. A page template is the URL with host, numbers, hex runs and query values masked, plus the masked title. |
| Frame log | For each template, how many observations there were and how many of them showed each line. It is counting evidence only; it does not decide that something is absent. |
| Episode store | Per episode a goal, an outcome and the action path with no-change and error flags. Episodes are searched by goal, outcome and path text. |

Purging an episode removes its steps, reverses its contribution to the effect table and frame log,
and frees every node, region and symbol that no other episode holds.

## When it helps

Observations are trees, consecutive and parallel episodes repeat large parts of them (navigation
menus, headers, the same page visited again), and questions are about what a page shows or what an
action changes. Then a region is scored once however often it recurs, and "what happens after
clicking Save on this page" is a table lookup.

It does not help prose. A paragraph has no subtree to share and no action to compare, so use the
document store.

## Prototype measurements

These are measurements of the Python research prototype on recorded agent trajectories over
accessibility trees (two environments, 228 development questions, gold-phrase coverage of the
retrieved text), not of this implementation:

- Regions retrieved +0.133 more gold-phrase coverage than chunked BM25 at an 8k-token budget
  (0.562 against 0.429) and +0.116 at 16k tokens (0.589 against 0.473).
- The unit store was about 5% of the raw tree text.
- Adding the effect table raised coverage on questions about changes by 0.067 to 0.133.

An end-to-end reader test with a compact pack did not beat the baseline reader on answerable
questions, so these are retrieval-coverage results and not an accuracy claim.

## Measured here

[`benchmarks/results/experience_ingest_synthetic.json`](https://github.com/writerslogic/warm/blob/main/benchmarks/results/experience_ingest_synthetic.json)
holds `cargo run --release --features experience --example experience_ingest_bench` on synthetic
trees (12 page types sharing a 30-link menu, 164 nodes per step, 30 steps per episode) on an 8-vCPU
x86 cloud VM. Tested range: 10, 100 and 1000 episodes.

| Episodes | Steps | Ingest, steps per second | Snapshot bytes per step | Raw tree text bytes per step | Snapshot / raw |
|---:|---:|---:|---:|---:|---:|
| 10 | 300 | 1574 | 1737 | 4984 | 0.348 |
| 100 | 3000 | 3056 | 457 | 4983 | 0.092 |
| 1000 | 30000 | 3723 | 108 | 4983 | 0.022 |

The ratio falls as pages repeat; it is a property of this synthetic repetition and not a
prediction for other data. Nothing was run past 1000 episodes.

## Rust API

```rust
use writerslogic_warm::core::experience::{Action, Experience, Observation};

let mut x = Experience::default();
x.describe_episode("run-1", "Cancel the pending order", "success")?;
x.ingest(&Observation { episode: "run-1", step: 0, tree: &tree, url: Some(url), action: None })?;
let action = Action::Call("click('12')".into());
x.ingest(&Observation { episode: "run-1", step: 1, tree: &next, url: Some(url2), action: Some(&action) })?;

let hits = x.search_regions("order status", 10)?;   // text, breadcrumb, provenance
let tid = x.template_id(Some(url), "Orders").unwrap();
let effects = x.effects(tid, None, 10);              // what actions on this page change
let episodes = x.list_episodes("cancel order", 5)?;
x.purge_episode("run-1");
x.save(dir)?;                                        // Experience::open(dir, cfg) reads it back
```

Ingesting a step again with identical content changes nothing (`Ingested::Duplicate`); a different
tree at the same step is `ExperienceError::Conflict`; a step before the last one is
`ExperienceError::OutOfOrder`. `ExperienceConfig` bounds depth (default 128, at most 512), nodes
per tree (200000), bytes per string (16384), episode id bytes (256), goal bytes (65536) and steps
per episode (1048576); a tree over a limit is rejected before anything is stored.

`Action::Call` takes a call such as `click('12')` and resolves the handle against the previous
observation of the same episode in this process; after a reload that map is empty and an
unresolved handle is labelled `bid 12`. Pass `Action::Label` to give the label yourself.

`detect_observation` recognizes a JSON object with `episode`, `step` and `tree` fields, and
`RegionHit::into_document_result` turns a hit into the `DocumentResult` the document store returns,
so a caller can merge both rankings.

## Persistence

`Experience::save` writes one checksummed snapshot atomically; see
[the format](../file-format/EXPERIENCE.md). It is a snapshot, not a journal: a crash loses the
steps ingested since the last save. The snapshot is not part of the document store's checkpoint.

## Not built

Retrieval keyed on surprise, absence detection from term coverage and vector-symbolic binding for
structure were measured to be dead in the prototype and are not here. The interval index for
timestamped values is specified but has no data to justify it and is not built.
