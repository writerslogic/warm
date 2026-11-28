# Changelog

All notable changes to this project are generated from the commit history.
Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) +
[Conventional Commits](https://www.conventionalcommits.org/).

[v0.6.3 migration and integrity notes](https://writerslogic.com/warm/docs/latest/releasing/v0.6.3/).
## [0.7.5] - 2026-11-28

### Added
- Add native in and eq filters, list matching, exact vector scores, model id and stemmer API (#558)
- Add a configurable master key path with a WARM_MASTER_KEY_PATH fallback (#552)
- Add atomic write_batch for mixed memorize and delete operations (#553)
- Read-only store opens beside one writer, and close() that releases the lock (#545)
- Publish the JavaScript adapters under the @writerslogic scope and depend on warm-memory (#538)
- Ship the Node engine as warm-memory with all eight bindings and the MCP launcher as warm-mcp (#533)

### Performance
- Store index-derived tables in the checkpoint and drop query-carried caches (#554)
- Screen exact vector scans and defer hit vector decoding (#547)
- Rank long deep unions from their strongest windows first (#542)
- Scan query lists from contiguous code copies for VIBE llama and yi (#541)

### Bench
- Count the warm-rust dependency tree and fail instead of recording zero (#560)
- Add a carried-state probe and its first run on the search-benchmark-game engines (#555)
- Add ANN mechanism ablation and paired SIFT/GloVe factor runs (#548)
- Add the same-host VIBE comparison of WARM, RoarGraph and MLANN on llama and yi (#544)
- Commit the long-union search-benchmark-game tables and exactness logs (#543)
- Commit the exhaustive differential log that the global ignore dropped (#540)
- Record that served BM25 top-k matches exhaustive scoring on the search-benchmark-game queries (#539)
- Add the pinned search-benchmark-game results for WARM 0.7.4 on c3-highcpu-8 (#534)
## [0.7.4] - 2026-11-26

### Documentation
- List the native Python package, restore cited durability results and repin the toolchain action (#530)

### Performance
- Speed up deep-k text ranking and serve vector indexes from huge pages (#531)
## [0.7.3] - 2026-11-26

### Documentation
- Add five standalone Node.js application examples (#519)
## [0.7.2] - 2026-11-25

### Performance
- Speed up long-union top-10, negated and multi-table rank paths; release 0.7.2 (#523)
- Tile k-means assignment and sum centroids in parallel (#522)
## [0.7.1] - 2026-11-24

### Added
- Learn reusable procedures from episodes and revise them on disproof (#509)
- Encrypt new stores by default with a plaintext opt-out (#507)

### Documentation
- Publish the docs site to writerslogic.com and fix stale package notes (#506)

### Performance
- Count conjunction candidates eight at a time (#520)
- Keep a conjunction's block bounds until a document passes their blocks (#518)
- Drop weak lone-phrase candidates before reading the other lists (#517)
- Serve lexical unions, conjunctions and phrases faster; release 0.7.1 (#512)
## [0.7.0] - 2026-11-18

### Added
- Parse form state from accessibility-tree text at experience ingest (#494)
- Run hybrid recall in the memory API (#475)
- Route tree observations through the default memory path (#472)
- Add the experience core for tree-structured agent observations (#468)
- Back memorize_vector with the vertex graph and add in-process Warm to the Python wheel (#440)

### Documentation
- Tighten docs, document and verify every adapter and example, publish workflows for the adapters, remove maintainer-only files (#502)
- Describe the editor plugins as on-device with no service calls (#492)

### Fixed
- Finish interrupted purges at open, purge by default with durable tickets, and grow segments on Windows (#453)

### Performance
- Probe candidate lists learned from construction queries before the graph search (#500)
- Reorder document slots by graph bisection in the explicit compaction (#495)
- Keep 512-bit gathers out of the block-wise conjunction loops (#488)
- Estimate 4-bit neighbour rows four at a time on the default query path (#484)
- Count intersections a block at a time (#476)
- Serve positional lexical queries from checkpoint version 4 with skip tables (#470)
- Make four-row batched neighbour estimates the default query path (#477)
- Bound the construction search and make standard the default build profile (#469)
- Add opt-in compressed checkpoint text (#473)
- Fold the text index to disk in bounded memory during ingest (#457)
- Add a binary vector query, queueing defaults and engine-side filtered search to warm-server (#471)
- Keep live-store posting lists decoded per thread for hybrid queries (#474)
- Serve large lexical corpora from the checkpoint with an exact impact-pruned ranker (#467)
- Add vector build profiles, run construction batches in locality order and release committed text in the text benchmark (#456)
- Speed up the streaming index with AVX-512 GEMM dispatch and packed selection keys (#463)
- Cut vertex build memory and store byte-valued rows as one byte per coordinate (#441)
- Weigh postings a block at a time in the lexical rank pass (#439)
- Speed up partition retrain placement and add block-sketch pruning to the sparse index (#437)
- Screen filtered IVF lists with 4-bit fast-scan codes (#434)

### Bench
- Add the hybrid contract (#466)
- Add vector baselines on a second machine class with a clean SIFT-1M final and GloVe (#458)
- Fold the three-machine text results, paired quality bootstrap, fact-store kill tests and LoCoMo context shapes (#464)
- Time every text engine on matched contracts, fix search_lexical, and fold the first-use, membership, sparse-docs and LoCoMo work (#454)
- Time hnswlib natively in C++ and report its Python call overhead (#444)
- Refresh the 185H competitive results at 5ed00fdc (#443)
## [0.6.8] - 2026-11-11

### Fixed
- Prepare the release with nonblocking tenant admission (#432)
## [0.6.7] - 2026-11-11

### Fixed
- Upgrade rmcp to 3.9 for RUSTSEC-2026-0462 (#420)
- Skip already-published npm versions so a partial release can be re-run (#418)

### Performance
- Speed up the filtered index and retune it for the yfcc-10M filter track (#417)
- Balanced warm retrain for streaming L22 (study and Ice Lake results) (#413)
- Honour rerank on a base with deletes; bench: compact LongMemEval context (#391)

### Bench
- Held-out search for the VIBE recall tail, learn-heavy builds and a rerank-dense grid (#410)
- LongMemEval dev context-assembly screen (partial, no 500 confirmation) (#407)
- LongMemEval_S 500-question held-out run (#394)
## [0.6.6] - 2026-11-09

### Added
- Expose the partition tier to Python and record the L22 full-stream run (#398)
- Add the experimental partition tier (IVF, SOAR spill, LUT16 residual codes) (#396)
- Write document checkpoints with block-packed postings (version 2) (#390)
- Land the supersession probes, k-mer, FPE timestamp and AVX2 popcount work (#388)
- Land per-store language analyzers, cheaper history views and a simulation harness (#387)
- Integrate Vectra filtering and validated retrieval work
- Add native vector migration and retire bincode (#378)
- Add experimental prefix retrieval and rename the Node SDK (#376)
- Expose verifiable audit records through public interfaces (#356)
- Recover committed audit effects after restart (#353)
- Name the Rust distribution warm-rust; complete example renaming and record registry retirement
- Use concise Python package names and distinct publishers
- Rename the product and integrations to WARM
- Verify scoped remote authorization and SCITT inclusion receipts (#340)
- Add organizational knowledge integration examples

### Documentation
- Bring the README and project docs up to date for 0.6.6 (#416)
- Record the Ice Lake streaming runs (recall 0.9848 in 2,666 s) (#404)
- Record the board-config streaming run (#399)
- Record the L22 partition-tier research, search profile and nprobe selection (#397)
- Fix broken server documentation link (#384)
- Qualify default graph recall
- Record ToIP collaboration introduction

### Fixed
- Bump oauthlib to 4.0.1 in the chroma benchmark requirements (#400)
- Skip embeddings for lexical search
- Embed fact-expanded document keys
- Validate direct staged search embedder identity
- Honor ingest spans in model pipelines; use local embeddings with model stages
- Bound stale graph delta capture
- Improve vector recall and harden storage boundaries
- Complete durable retry and query-efficiency followups (#375)
- Complete recovery, retrieval and native binding readiness (#374)
- Propagate checked Memory API reads (#372); preserve checkpoints during vector maintenance (#373)
- Propagate checkpoint metadata read failures (#370); bound MCP exports and select text reads (#371)
- Delete aged CrewAI memories across all pages (#368); propagate checked document read failures (#369)
- Secure maintained integration dependency profiles (#367)
- Retire legacy SDK integrations and refresh dependencies (#365)
- Tighten document summaries and Haystack contracts (#363)
- Harden archive restores and dependency exposure (#360); enforce Python framework adapter contracts (#362)
- Correct HTTP errors and expand Windows service coverage (#359)
- Reject ambiguous authentication and retry headers (#358)
- Integrate scoped recall and Windows spill review repairs (#347)
- Allow legacy npm deprecation without deletion retries
- Preserve the MCP reserved metadata contract in release schemas
- Use the accepted warm-python distribution name; record WARM publication and bound external link checks
- Preserve durable identities and bound security lifecycle failures (#341)
- Share memory validation and preserve adapter ownership (#339)
- Resolve temporal eligibility before memory ranking (#338)
- Resolve additional bounded audit concerns (#335)
- Resolve audited integrity and retrieval defects (#334)

### Performance
- Test the low-level system defaults of the L22 streaming path (#409)
- Cut the L22 scan overhead around the quad kernel and batch the reorder (#406)
- Fuse admission into the LUT16 scan and add AVX-512 and VNNI quad paths (#403)
- Build the upper layers before the adjacency during consolidation (#393)
- Decode checkpoint hits without a chunk vector and fuse the streaming rescoring pass (#392)
- Land AVX2 screen popcount, background compaction and historical views (#386)
- Land working tree, journal segment sizing and query hot lists (#385)
- Avoid copying encoded vectors during reads
- Reduce vector query overhead
- Reduce retrieval capture allocations and validate audit write cost (#354)
- Prune inactive documentation navigation; install the verified WARM registry releases
- Preload the documentation landing image; enable protected publication of renamed npm packages
- Reuse filtered eligibility and bounded dense mutation deltas (#337)

### Bench
- VIBE OOD check, and the streaming entry's Ice Lake timing (#401)
- Pin the text comparison and speed up mapped-checkpoint posting decode (#389)

### Release
- Publish the MCP Python launcher as warm-mcp
## [0.6.4] - 2026-10-28

### Changed
- Organize bindings and research source modules

### Documentation
- Align landing page with retrieval architecture
- Add Glama score badge (#316)
- Prepare npm listing and runtime release (#318)

### Fixed
- Bootstrap Python docs from the current source layout
- Synchronize release and OpenAPI versions
- Preserve guarded final benchmark allocations
- Enforce provenance semantics and cached status expiry
- Preserve Mem0 recovery and completion integrity
- Clarify MCP tool behavior for Glama (#317)
- Reduce crate archive for crates.io (#321)
- Verify every npm platform before install (#320)
## [0.6.3] - 2026-10-25

### Added
- Make MCP server scorable by Glama (#313)
- Add label-filtered exact uint8 search with per-label inverted files (#291)
- Add compact adjacency and publish ANN cost results (#206)
- Diagnose streaming recall and verify crash boundaries (#201)
- Add exact Boolean lexical search and blocked postings (#200)
- Batch inserts and a Python UpdatableIndex for the updatable vector graph (#190)
- Count matching documents and ingest unstemmed terms (#193)
- Preserve original conversation evidence and audit memory quality (#184)
- Euclidean and inner-product metrics in the Python VectorIndex (#186)
- Clustered private ranking of dense codes at n = 1280 (#177)
- Add NGT, ScaNN, PyNNDescent, Voyager and DiskANN to the ANN harness (#156)
- Expose the vector index to Python and add ann-benchmarks and VIBE wrappers (#149)
- Stable C ABI with Go, Java and .NET bindings (#109)
- Index-routed server vector queries, deferred ingest modes, memoized encoder, and round-2 research kill tests (#139)
- Local embeddings (pure-Rust engine, pinned model packs, LocalEmbedV1) (#132)
- WebAssembly build of the document store (@holographic-memory/wasm) (#129)
- Production hms-server (scoped auth, observability, TLS, snapshots, deploy artifacts) (#127)
- UniFFI mobile bindings (Swift, Kotlin/Android) (#121)
- Memory API with temporal validity, supersession, explain and opt-in policies (#114)
- Hms-import, one-command migration into HMS (#108)
- Platform kernels (AVX512-VNNI, NEON fallback) with kill-test results (#107)
- Integrate the parallel sessions' document store, compaction and server work (#81)
- Patience stop rule, opt-in 1-bit screen and error-returning builds for the vertex graph index
- Competitor ANN harness for SymphonyQG, RaBitQ, NGT and Lucene HNSW
- Check the graph cache header and verify the merged qgraph defaults
- Add on-device Qwen3 model stages behind local-models; update changelog
- Add quantized graph ANN index and its public benchmark mode; record quantized graph results against HNSW
- Add experimental query-private similarity scoring behind private-search
- Subset-union bundle experiment and capacity sweep; route sparse queries to the exact inverted index

### Changed
- Prune uncited experimental research code (#210)

### Documentation
- Polish MCP package pages and link website (#311)
- Make source quickstart accurate
- Refresh branding and improve integration and search guides (#308)
- Preliminary nytimes VGraph comparison on the M4 and the next-session specs; update changelog
- Standardize README header (#71)
- Attribute semantic search to the document API
- Update changelog [skip ci]; add public BEIR and ann-benchmarks comparisons
- Update changelog [skip ci]

### Fixed
- Keep Python source archive within PyPI limit (#314)
- Rename Python packages and polish PyPI pages (#310)
- Update benchmark harnesses for current Clippy
- Mark the gzip LFS exception as binary (#225); read vector result text under the store lock (#227)
- Judge recall status from the index of the revision searched (#168)
- Make each journal commit durable on macOS with F_FULLFSYNC (#151)
- Purge physically erases deleted text (#124)
- Satisfy clippy 1.99 lints in the qgraph kernels and graph cache
- Resolve leftover merge markers in .bestpractices.json; drop rejected intersection kernel candidates
- Return query results best-first on exact-scan, multi-shard and federated paths

### Performance
- Accelerate lexical query execution and decoding
- Improve phase memory and recover conversation premises
- Reduce filtered-search allocation and scoring overhead
- Share immutable position sources by document slot (#305)
- Spill delta rows during vector consolidation (#286)
- Avoid copying surviving vectors to replace a deleted graph entry (#282)
- Skip inverted-index hits no better than the worst kept one (#277)
- Return inverted-index hits as slot numbers instead of decimal strings (#267)
- Fuse the first three Walsh-Hadamard stages per block of eight (#260)
- Clear the streaming delete-runbook baseline and drop the insert row copy (#238)
- Huge-page advice on by default, with counters and a refusal fallback test (#241)
- Build chunk ids at their exact size and check them without allocating (#253)
- Single-thread vertex search (prefetch terms, SIMD re-rank, 8-bit codes in the harness) (#226)
- Persist the hms-server vector graph and open it at start (#228)
- Qgraph review fixes (#220); read search result text under the mutation gate (#223)
- Full-text engine and storage review fixes (#218)
- Eliminate redundant graph construction and query work (#209)
- Store construction adjacency in one fixed-slot array (#182)
- Faster single-threaded text ingest (#178)
- Lean BM25 search endpoint and in-place fact replacement in hms-server (#176)
- Serve the vertex index from a memory-mapped file (#172); build the vertex graph faster (#174)
- Rank unfiltered lexical queries in a per-thread slot table (#154)
- Round-3 ingest, memory and ANN work with research results (#143)
- NEON per-row 1-bit screen kernel with its microbenchmark kill test (screen stays off); update changelog
- Gallop before AVX2 on skewed sizes; add intersection kernel bench

### Merge
- Integrate vector search improvements
## [0.6.1] - 2026-10-05

### Added
- Introduce IdentityRegistry enforcing W3C DIDs, Verifiable Credentials, and POSME Receipts for queries
- Add Phidata integration and fix broken Rust examples
- Pivot to Zero-Trust Edge Architecture (FHE-Lite, AGPL-3.0, SIMD)
- Python bindings + PyPI packaging for holographic-vsa (maturin/pyo3)
- PhaseResonator reusable-index API + clean re-export + doctest (§20 capability)
- Counting membership store with Poisson z-score readout (4x Bloom capacity)
- Real SHA-256 commitment for verifiable phasor mutation (lane 3)
- Multi-hop reasoning over the phasor memory (retrieve_path); update changelog [skip ci]
- PhaseGraph -- phasor relational memory with relation algebra + retrieval
- Persist ConnectionGraph via its event log; survives restart; update changelog [skip ci]
- ConnectionGraph -- plastic event-sourced relation store, wired into engine; update changelog [skip ci]
- Trust-anchored provenance verification and sharded ANN persistence
- Runnable cross-verification of the cogmem C2PA sample
- Add provenance system with COSE, VCs, C2PA, JUMBF, Sigstore, KERI, CAWG
- Add research-grade benchmarks, scaling analysis, and visualizations
- Add sparse Clifford algebra multivector type (CliffordVec)
- Add Hopfield-Fenchel-Young energy-based associative retrieval
- Multi-scale encoding, morphological decomposer, fault-tolerant federation
- Improve SOTA for analogy, planner, and concept synthesis
- Add cost/total_cost fields to planner N-API types
- Evidence-weighted multi-hop confidence, ranked results
- Expose cleanup_vector N-API method for standalone Hopfield denoising; cargo fmt
- Add agency layer (GoalStore, Planner, QuestionGenerator, SelfModifier)
- Add cognition layer, distributional refiner, CI coverage; cargo fmt formatting
- Integrate meaning memory into HmsCore, wire write/delete paths (G5)
- AtomMemory, CompositeMemory, TripleStore, RuleStore, Decomposer (G3)
- Shared utilities + IndexedMemory with Hopfield attractor (G1-G2)
- Rename to holographic-memory, fix npm logo, update all URLs
- Graph engine with multi-hop traversal, inference, temporal, federated queries
- Complete security integration, DP wiring, docs, LF readiness
- Add security features behind feature flag for LF Decentralized Trust readiness
- Migrate to writerslogic org, add npm publish pipeline
- Eliminate remaining gaps, expose train_nsg/train_ivf via N-API
- Expose diffusion config through HmsConfig and N-API
- Configurable thresholds, N-API config constructor, status getters
- Add query_sequence, elevate test coverage for weak features
- Add multi-shard support with auto-sharding
- Initial release of Holographic Memory System (HMS)

### Changed
- Remove dead self-inverse resonator; move ResonatorConfig to phase_resonator
- Extract meaning_ctx() helper shared by structural_query and multi_hop
- Gate experimental VSA modules behind experimental feature
- Migrate HMS agent identity from did.cose to CAWG ICA
- Deduplicate RNG pruning, unify patterns, add doc comments
- Eliminate dead code, add delete/compact with persistence
- Remove redb dependency, use in-memory FxHashMap with arena log persistence

### Documentation
- Update changelog [skip ci]
- State the AGPL-3.0-or-later license in the security policy
- Update changelog [skip ci]; release 0.6.1
- Rewrite README to highlight Zero-Trust Enterprise Security and Python SDK
- State the testing policy explicitly in CONTRIBUTING.md (#66)
- Sync changelog (#54)
- Align guidance and track deferred production work (#52)
- Update changelog [skip ci]
- Update changelog [skip ci]
- Standardize badges (add OpenSSF Scorecard; consistent order/format)
- Update changelog [skip ci]
- Update changelog [skip ci]
- Update changelog [skip ci]; update changelog [skip ci]
- Update changelog [skip ci]
- Update changelog [skip ci]
- Update changelog [skip ci]
- Update changelog [skip ci]
- Update changelog [skip ci]
- Update changelog [skip ci]
- Update changelog [skip ci]
- Update changelog [skip ci]; story_memory -- HMS reasoning memory serving a writer (scrivener-mcp)
- Update changelog [skip ci]; update changelog [skip ci]
- Update changelog [skip ci]; update changelog [skip ci]
- Update changelog [skip ci]
- Update changelog [skip ci]
- Update changelog [skip ci]; update coverage badge [skip ci]
- Update changelog [skip ci]
- Update changelog [skip ci]
- Update changelog [skip ci]
- Update changelog [skip ci]
- Restructure README with collapsible sections
- Update changelog [skip ci]
- Update changelog [skip ci]
- Rewrite README — fix logo for npmjs, restructure with quick start first, improve clarity
- Update changelog [skip ci]
- Update changelog [skip ci]
- Update changelog [skip ci]
- Update changelog [skip ci]
- Add agent-provenance stack cross-reference to README
- Update changelog [skip ci]
- Update all documentation for v0.5.0 accuracy

### Fixed
- Remove unimplemented security claims and retired cloud code
- Add missing experimental feature gate to zero_trust_demo
- Resolve CodeQL alerts and README formatting
- Resolve clippy::chunks_exact_to_as_chunks lint regression on main (#61)
- Resolve workflow code scanning alerts (#53)
- Give relations a distinct magic byte (0xFA), accept legacy 0xFE on read
- Close remaining provenance trust-anchor gaps (cawg, vc-only records)
- Use sort_by_key to satisfy clippy 1.96 unnecessary_sort_by
- Repair --all-features build (port SCITT to ureq 3 API, drop dead CliffordVec bench, gate provenance example)
- Allow CDLA-Permissive-2.0 license from webpki-roots via ureq
- Replace bare unwrap() with expect() or total_cmp in production code
- Remove dead CliffordVec code and fix clippy warnings
- Perf and safety improvements from audit; update coverage badge [skip ci]
- 5 bugs from audit (deadlock, double-decay, over-count, double-start, stale-used)
- Use sort_by_key for clippy on newer Rust
- Resolve all clippy warnings for CI
- Quality pass on G1-G4 code
- Resolve 15 audit findings (3 critical, 8 high, 4 medium)
- Crash-safety, error propagation, and compaction correctness
- Propagate errors from shard insert/remove, guard ShardManager invariant
- Use .clamp() instead of .max().min() to satisfy clippy

### Performance
- Optimize Zero-Trust FHE-Lite key generation and eliminate string cloning in hot loops
- Use posting-list candidate generation for synthesizeConcepts (>500 vectors)

### Security
- Remediate repository alerts and dependencies (#38)
- Harden signing-key file perms, zeroize copies, bound JUMBF recursion

### Assets
- Static transparent logo from first frame, text removed
- Replace logo with self-contrasting node-graph mark (animated gif + static svg), drop garbled-text variants

### Deps
- Land 10 Dependabot major bumps with API fixes (#26)

