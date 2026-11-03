# Reproduction status

Milestone 8 is open: no independent evaluator has reproduced a qualified system
advantage. Windows and WSL are one physical computer; the Linux guest on the Mac
also adds no physical hardware. An independent rerun is an evaluator requirement,
separate from the two-hardware requirement. The user volunteered on 2026-11-03 to reproduce the results. Prepare the pinned
source, inputs, executable commands and expected gates for their run; retain their
returned raw results separately from agent-generated checks. Milestone 8 remains
open until that run reproduces a qualified claim. No external messages or
publication have been sent.

## Portable research artifact available now

The temporal decision-memory experiment has a self-contained source bundle:

`/Volumes/A/.hms-bench-cache/warm-temporal-reproduction-20261103-v1/temporal-reproduction-v1.tar.gz`

Archive SHA256:
`902d1860acf7e47532d6ce24426bbbb479248dce6431172a378df845d586a412`.
Manifest SHA256:
`af5938f4d72d4c862893fc8baf0295901cd7ca38d6dac04fd3c98eb4a1a81b24`.
The archive contains ten source/documentation files plus its manifest; it is
21,556 bytes. After extracting it, run from its root:

```sh
uv run --no-project --python 3.12.12 python benchmarks/research/temporal_trace/reproduce.py --bundle "$PWD" --output "$PWD/replication-001"
```

The output directory must not exist. No model, paid service, server or external
corpus is used. uv may obtain the specified free Python runtime when absent.
The runner checks source hashes, runs nine tests, regenerates every temporal
case, compares all twelve cohort packets byte-for-byte to the original evidence,
and reruns the established symbolic control. It retains complete output and
failure logs. Timing values are recorded but need not match across machines.

Expected scientific outcome: 3,932 cases and 86,846 snapshot-withdrawal checks
preserve decisions and read provenance; the size screen fails in four of eight
required cells; the symbolic control produces identical packets in all twelve
larger cases. Reproducing this negative result is the intended success condition,
not an algorithm-superiority claim. See [TEMPORAL-TRACE.md](TEMPORAL-TRACE.md).

A clean extraction was tested locally using Python 3.12.12. It reproduced all
counts and packet hashes and passed all nine tests. The receipt explicitly says
`independenceVerified: false`; agent packaging verification is not independent
evaluation. Logs and receipt are `temporal-reproduction-check.{log,json}`, with
the complete rerun under the artifact root's `packaging-check/` directory.

## Portable deletion-bound artifact

A second portable artifact reproduces the positive-evidence deletion-bound
construction in [WITNESS-TIGHTNESS.md](WITNESS-TIGHTNESS.md):

`/Volumes/A/.hms-bench-cache/warm-witness-reproduction-20261103-v1/witness-reproduction-v1.tar.gz`

Archive SHA256:
`7e200ab655cb18c2e0f8db9838fbe6a8ecc50cd9969aba179815549cf66c1967`.
After extraction, from the bundle root:

```sh
uv run --no-project --python 3.12.12 python benchmarks/research/source_prediction/reproduce_witness.py --bundle "$PWD" --output "$PWD/replication-001"
```

The 53,247-byte archive contains pinned source, proof, original profiles/readouts,
expected hashes and instructions. A fresh local extraction reproduced all 11
families / 8,188 comparisons, five existing-checker witnesses, three focused tests
and thirteen byte-identical evidence files. A deliberately changed source byte
was rejected before computation. Receipt: `witness-reproduction-check.json`.
This is a mathematical construction, not a system-advantage result; its local
packaging check is not independent evaluation. Return the complete new output
directory with machine/runtime details and the verified archive hash.

## Portable wholly withdrawable deletion-bound artifact

The stronger result in [MATRIX-WITNESS.md](MATRIX-WITNESS.md) also has a complete
proof/source/raw-evidence bundle:

`/Volumes/A/.hms-bench-cache/warm-matrix-witness-reproduction-20261103-v1/matrix-witness-reproduction-v1.tar.gz`

Archive SHA256:
`62e759b59265a119909a5d5dd0cd147e5fd11f5f4d1402250bf238cc1b702a8f`.
Manifest SHA256:
`f415482878091edb0d8b1821e3eb2fd182e847f9e6a61e07421ea5416af0e965`.
After extraction, use the same `reproduce_witness.py` command above. The
365,017-byte archive contains 23 manifest-listed files. A fresh local extraction
passes three focused tests and reproduces all five families / 37,448 comparisons,
62 distinguishing masks, 17,841 jointly unknown masks, three existing-checker
cases and seven byte-identical output files. A changed source byte is rejected
before computation. Receipt: `matrix-witness-reproduction-check.json`.
The earlier bundle remains intact. This is another agent packaging check;
independent evaluation and a system-advantage claim remain separate requirements.

## Portable source-count refinement

The source-count refinement has its own portable bundle:

`/Volumes/A/.hms-bench-cache/warm-ambient-witness-reproduction-20261103-v1/ambient-witness-reproduction-v1.tar.gz`

Archive SHA256:
`4a9327b9fb6d7768fa253e2055fbc7a04731d33dfd545c955e4cfba7c3994292`.
Use the same `reproduce_witness.py` command above from a fresh extraction.
The 330,046-byte archive contains 51 manifest-listed files. Its clean local
extraction passes four tests and reproduces all 32 families / 37,410 comparisons,
408 distinguishing masks, 14,346 jointly unknown masks, 21 existing-checker
cases and 34 byte-identical outputs. Changed source bytes are rejected before
execution. Receipt: `ambient-witness-reproduction-check.json`; result and proof:
[AMBIENT-WITNESS.md](AMBIENT-WITNESS.md). This establishes the scoped
`min(2g,n−g)` distribution-witness tradeoff, not system superiority or independence.

## System-claim reproduction remains unfinished

The system bundle must include the exact buildable source and overlays, pinned
toolchains/dependencies, input provenance and preparation hashes, frozen policies
and calibration selections, all raw quality/performance/recovery observations,
and failures. A source commit alone is insufficient because the qualified
development artifacts include the uncommitted durable-receipt correction.

Current evidence is indexed in [STATUS.md](STATUS.md), [RECALL.md](RECALL.md),
[ACCURACY.md](ACCURACY.md), [PERFORMANCE.md](PERFORMANCE.md),
[MEMORY-UTILITY.md](MEMORY-UTILITY.md) and [PLATFORMS.md](PLATFORMS.md).
The frozen 24-hour soak, full serving comparison, real utility and remaining
platform qualifications must finish before that bundle can support the requested
system claim. Research packet savings and symbolic toy correctness do not replace
those requirements. No independent pass or competitive advantage is recorded.

The completed serving evidence export and its SHA256 are recorded in
[PERFORMANCE.md](PERFORMANCE.md). All 18 raw audits pass, but the comparison
fails quality/advantage gates, its original process accounting misses PostgreSQL
children, and host isolation is unqualified. The export is not a portable
source/input reproducer or an independently reproduced system claim.
