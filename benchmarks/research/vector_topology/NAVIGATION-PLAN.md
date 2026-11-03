# Original recall failure attribution

This local development diagnostic explains the already observed ef=128 and ef=256
failures on the exposed 10,000- and 100,000-row, 100-dimensional GloVe corpora and all 1,000
queries. It does not qualify held-out accuracy, timings, or a competitive claim.
No paid service is used. Preserve original stopped-study artifacts.

Build the default degree-32/build-128 graph, seed and encoding unchanged. Require
the 100K structure hash to equal the previously reproduced `b032161111580eb9` and
each ordered final top ten to equal the original saved result for that ef/query.
Require the original exact results to equal the independently saved oracle.
Use the existing normalized f32 storage, f64 final rescore, 18-candidate shortlist,
and residual rerank of the full ef beam.

Export directed base-layer adjacency decoded from the serving index. For every
query record the post-descent entry, each beam offer with score and cutoff,
admission/rejection, evicted vertex, expansion, final beam, and stage coverage.
The diagnostic replay must match serving search's complete visited set, ordered
beam, expansion count and results; fail immediately on mismatch. The plain
serving search loop is unchanged. Runtime graph knobs, screens, blocks,
independent reservoirs, patience and neighbor caps are excluded from this control.

Independently replay the event stream in JavaScript, validating best-first
expansion, adjacency/visited ordering, admission thresholds, eviction and complete
beam exhaustion. Run directed BFS from each entry without a beam bound. For every
missing oracle neighbor report reachability, every incoming neighbor's state,
a shortest-path witness and the first unexpanded bridge's admission/eviction
history. Unreachable targets are topology loss; reachable but undiscovered
targets are bounded traversal loss, with rejection versus pre-expansion eviction
identified on the witness path. The witness is sufficient to show a valid path
was blocked, not a claim that this path is the only possible route. Later-stage
losses remain distinct. All failures and zero-miss queries are retained.

Use one fresh output, no automatic retry, existing two-hour/8-GiB monitored
process limits, and one Rayon/BLAS thread. Record source revision, patch, new
source files, lockfiles, runtime, binary and input hashes, raw events, compressed
events, per-query miss explanations and a failure-inclusive receipt. This run
does not time search and need not claim a quiet host.

```sh
CARGO_TARGET_DIR=/absolute/new-build cargo test --offline --locked --lib --features experimental core::qgraph::vertex::navigation
CARGO_TARGET_DIR=/absolute/new-build cargo build --release --offline --locked --manifest-path benchmarks/research/vector_topology/Cargo.toml --features writerslogic_warm/experimental --bin trace
node --test benchmarks/research/vector_topology/navigation-test.mjs
env -u NODE_OPTIONS node benchmarks/research/vector_topology/navigation-run.mjs /absolute/new-output /absolute/new-build/release/trace
```
