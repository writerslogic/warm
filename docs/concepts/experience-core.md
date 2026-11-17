# Experience core

The experience core is memory for observations that are trees and that repeat: the accessibility
tree of a web page an agent looks at after every action, a UI hierarchy, a directory listing. It is
part of the engine and always built: the memory API's `remember` sends an observation to it by
shape, and `recall`, `explain` and `forget` cover it (see below). A store that never receives an
observation has no experience files, and search over documents and facts is not changed by it.

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
per tree (200000), bytes per string (16384), episode id bytes (256), goal bytes (65536), steps
per episode (1048576) and accessibility-tree text bytes (4 MiB); a tree over a limit is rejected
before anything is stored. Its `form_state` switch is described under [Form state](#form-state).

`Action::Call` takes a call such as `click('12')` and resolves the handle against the previous
observation of the same episode. The handle map of each episode's latest observation is stored in
the snapshot, so a call after a reload resolves as it would have without one; a version 1 snapshot
has no map, and its first call after opening labels an unresolved handle `bid 12`. Pass
`Action::Label` to give the label yourself.

`detect_observation` recognizes a JSON object with `episode`, `step` and `tree` fields, and
`RegionHit::into_document_result` turns a hit into the `DocumentResult` the document store returns.

## On the memory path

No flag and no separate API are needed.

- `remember`: a request (or an entry of `facts`) that is an object with an `episode` string, a
  `step` number and a `tree` object is an observation: `{"episode", "step", "tree", "url"?,
  "action"?, "goal"?, "outcome"?, "accessibilityTree"?}`. `tree` nodes are `{"role", "name", "props"?, "handle"?,
  "children"?}`. Everything else is a fact and is handled exactly as before. Facts in one call are
  stored in one transaction first; observations follow in order, each durable when the call
  returns, and a failing observation leaves the ones before it stored. `Memory::remember_observations`
  is the typed form. Depth, node count and string bytes are checked on the JSON value before a
  tree is built from it (`ExperienceConfig`), and `warm-server` checks them before the core sees
  the request. Over HTTP the JSON parser refuses nesting past 128 levels, and a tree level takes
  two (an object and a `children` array), so a request can carry 63 tree levels; the depth limit
  of 128 applies to the library and the bindings. A request nested deeper than the parser allows is
  refused with 400 `nesting_too_deep`. `warm-server`'s `--max-documents-per-tenant` counts one unit
  per stored document plus one per episode, region and effect row; a step that would pass it
  gets 429 `quota_exceeded` and leaves nothing stored (steps before it in the same call stay).
  The quota counts entries, not bytes. The shape and limits of every entry in a batch are
  checked before anything in it is stored; an out-of-order or conflicting step is still found only
  when it is applied.
- `recall`: the best regions and the best action-effect rows merge with the fact results into one
  list. Regions, effect rows and facts are ranked by three lexical indexes (the effect index holds
  each row as rendered text, `On page 'P': click X (x3) -> now shown: ...`); they have separate
  corpus statistics, so their scores are not on one scale and are not compared. The merge uses
  ranks only: a result at position `r` of its list contributes `1 / (60 + r)`, results are ordered
  by that value (a fact before a region on a tie, a better rank before a worse one) and `score` is
  the merged value. A recall without regions, or one that sets `subject`, `entities`, `filter`,
  `knownAt` or historical mode (which regions do not have), returns the fact results unchanged,
  with their own scores. Region results have the id `experience:r<N>`, the page text and
  breadcrumb as `text`, and `metadata.provenance` listing up to eight `{episode, step, count}`.
  Effect rows have the id `experience:x<template>:<action>` and the rendered row as `text`; their
  provenance lists the first transitions that produced the row (found by scanning the stored
  steps, so `explain` on an effect row is linear in the stored steps).
  Region ids are stable while the store stays open and across a snapshot reload; a purge followed
  by new observations can reuse a freed id.
- `explain`: for an id `experience:r<N>` or `experience:x<template>:<action>`, rank among the best 100 regions, lexical score, matched
  terms and `provenance`.
- `forget` with `{"kind": "episode", "episode": "<id>"}` removes the episode and everything only it
  held. It is always a scrubbing purge: the purge is journaled, the snapshot is rewritten without
  the episode, and the old snapshot and journal are overwritten with zeros before the call
  returns (`dryRun` lists what would go; `purge` and `synchronous` do not apply). A region that
  other episodes also show is kept, but what the episode contributed to it (its value variants and
  the page title, breadcrumb and text taken from its first occurrence) is re-derived from the
  episodes that remain, so no text of the forgotten episode stays in the snapshot.

## Form state

An observation can carry the accessibility-tree text of its page in `accessibilityTree`
(`OwnedObservation::accessibility_tree`). At ingest the engine parses the form controls from it
and stores them with the step; the text itself is not stored. A control is a node whose role is
`textbox`, `searchbox`, `combobox`, `listbox`, `checkbox`, `radio`, `switch`, `spinbutton`,
`button`, `tab`, `menuitemcheckbox` or `menuitemradio`, read from lines of the form
`[id] role 'name' props`, with the properties `value`, `checked`, `selected`, `expanded`,
`pressed`, `required`, `disabled` and `readonly`; a combobox or listbox also keeps the `option`
nodes indented below it, the selected ones marked, the first 25 by name and the rest counted.
An unnamed button with no state is not kept. Nothing in the parser names a site.

At query time `Experience::form_tables` (and `WarmCore::experience_form_tables`, Python
`Warm.experience_form_tables`) renders the controls of a list of steps, one block per step:

```text
- Form controls (structured from the page text; * = selected option):
  combobox 'Sort by' value='Newest', expanded=False options: *Newest | Oldest | Price
  textbox 'Email' value='a@b.c', required
  [3 controls unchanged from an earlier state omitted]
```

A row already shown for an earlier step of the same list is left out and counted, and a step
without controls gives an empty string. The rendering is the reference renderer of the
LongMemEval-V2 adapter (`benchmarks/lme2/`); `tests/experience_form_state.rs` compares the two on
28 recorded states of the benchmark's public trajectories, byte for byte. Names and values are
quoted as Python's `repr` quotes them, which is what the reference does.

Bounds: the text is at most `ExperienceConfig::max_page_bytes` (4 MiB) and is refused over it
before anything is parsed; a line longer than `max_text_bytes` (16 KiB) is skipped; a step keeps
at most 120 controls and a control at most 25 options; a line that does not parse is skipped.
`Experience::ingest_with_controls` takes controls a caller built itself under the same bounds.
The controls are part of a step's content: the same step sent again with other controls is a
conflict.

`ExperienceConfig::form_state` is on by default. It changes nothing for a store whose
observations carry no `accessibilityTree`: such a store writes the same journal records and the
same version 2 snapshot as before. Off, the text is dropped unread. The parsed controls are
journaled with the step (an `IngestForm` record) and saved in a version 3 snapshot, so a reopen
does not parse again and does not depend on the switch.

## Persistence

Experience state lives in the store directory (`experience.wexp`, `experience.wexj`), created on
the first observation. Each accepted change (a step, an episode description, a purge) is one
journal record, synced before its call returns; a checkpoint writes the snapshot and empties the
journal. Checkpoints run with every document checkpoint (`compact`, background compaction),
with `flush`, when the journal passes 64 MiB, and after a purge. In an encrypted store both files
are sealed with the store's key. Opening a store replays the journal over the snapshot, drops a
record the crash cut short, and finishes an interrupted purge. See
[the format](../file-format/EXPERIENCE.md). The standalone `Experience::save` and `open` still
write and read a snapshot with no journal.

Not measured or not done: the whole snapshot is rewritten at each checkpoint, so its cost grows
with the store; only a kill of the process is tested, not power loss; scrubbing overwrites files in
place and cannot reach copies that a copy-on-write or wear-levelled disk keeps.

## Not built

Retrieval keyed on surprise, absence detection from term coverage and vector-symbolic binding for
structure were measured to be dead in the prototype and are not here. The interval index for
timestamped values is specified but has no data to justify it and is not built.
