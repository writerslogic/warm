# Mission development evidence

This package records local development counter results and partial instrument audits. No row meets the mission confirmation target. Production engine code is unchanged from the parent revision.

Canonical ongoing state lives at `/Volumes/A/.hms-lb/mission/`; `ledger/` is a point-in-time snapshot. The matrix remains incomplete: unresolved inventory rows in A/B/C remain required work. TSV snapshots use quoted cells to preserve empty fields without trailing whitespace.

E3 checks external LoCoMo audit provenance; E7 independently reviews five source claims. These are partial proposals, not scored results. E4 rejects global scalar interval rescue. E5 rejects one-round base-neighbor completion. Retain complete grids and failures.

Each lane's `sources.tar.gz` preserves fetched source bytes and its original checksum manifest. Extract into its lane directory with `tar -xzf sources.tar.gz`; then run the corresponding script using its `--help` and the registered arguments. Inputs for the vector counters are existing development datasets identified by hashes in result files. They are not untouched holdouts. No paid service is needed.

`artifact-sha256.json` records all packaged file hashes except itself. Counter registrations retain source and script hashes. Engine final gates have not been rerun for this research-only package; prior engine validation is recorded in the parent candidate, and integrated confirmation remains outstanding.
