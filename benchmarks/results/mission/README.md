# Mission development evidence

This package records local development counter results and partial instrument audits. No row meets the mission confirmation target. Production engine code is unchanged from the parent revision.

Canonical ongoing state lives at `/Volumes/A/.hms-lb/mission/`; `ledger/` is a point-in-time snapshot. The matrix remains incomplete: unresolved inventory rows in A/B/C remain required work. TSV snapshots use quoted cells to preserve empty fields without trailing whitespace.

E3 checks external LoCoMo audit provenance; E7 independently reviews five source claims. These are partial proposals, not scored results. E4 rejects global scalar interval rescue. E5 rejects one-round base-neighbor completion. Retain complete grids and failures.

E14/E15 pass numerical refinement-count gates for byte reconstruction certificates on two exposed synthetic distributions. E19/E20 pass the same exact-top-ten checks using a fixed seed policy without oracle thresholds. E24 passes 2,308 independent rational checks of the scalar interval reference. Native arithmetic, official score parity, memory and throughput remain unproved. E21 rejects fixed-eight packetization on the smaller archived corpus.

E23 verifies all 156 externally flagged LoCoMo reviews and 1,069 source references, including the two previously accepted source corrections from E7. This does not audit the other 1,384 nonadversarial questions or 446 adversarial questions. E25 finds duplicate session IDs in 13 LongMemEval questions; the stored official and inline judge files share 21 persistent negative labels. E27 shows those 13 duplicate groups share reader text but carry different dates; only one appears in the historical top-30 ranking and none belongs to the persistent 21. Historical final-occurrence lookup loses temporal provenance. No audited score has been computed.

Each lane's `sources.tar.gz` preserves fetched source bytes and its original checksum manifest. Extract into its lane directory with `tar -xzf sources.tar.gz`; then run the corresponding script using its `--help` and the registered arguments. Inputs for the vector counters are existing development datasets identified by hashes in result files. They are not untouched holdouts. No paid service is needed.

From this directory, use `uv sync --locked` for the pinned Python dependencies. `B/reproduce_development_inputs.py --help` documents regeneration of the synthetic inputs; E13/E18 verify both noise settings against the original data hashes. Run `uv run B/check_interval_reference.py --output interval-check.json` to reproduce the scalar proof fixtures. Other registrations retain the full argument lists; replace host-specific paths with the matching checksum-pinned local inputs. A's `ann-sources.tar.gz` holds the additional archived ANN leaderboard evidence; its recovered maxima have unresolved currency and are not accepted as current comparators.

`artifact-sha256.json` records all packaged file hashes except itself. Counter registrations retain source and script hashes. Engine final gates have not been rerun for this research-only package; prior engine validation is recorded in the parent candidate, and integrated confirmation remains outstanding.
