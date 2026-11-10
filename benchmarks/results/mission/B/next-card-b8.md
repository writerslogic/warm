# H-B-8 fixed upper-seed necessary counter
Limitation: H-B-5's4096exact seed distances/query may consume the remaining runtime budget.
Mechanism: unchanged4096SplitMix64 live-tag seeds; reconstructed-distance upper bounds provide incumbent upper thresholds; fixed4096-row scan refines admitted rows and resolves final ranking exactly.
Prediction: all actual raw refinements, including admitted seed rows, have mean<=2048 and samplep99<=8192 with zero bound/upper-seed/prune/parity failures.
Protocol: existing1M-row easy source, first16queries; hard source unchanged only after easy passes;600second cap, no seed/code/order sweep.
Numerics: reuse frozen E36 native metadata and uint128 distances where supported; exact Fraction fallback; vectorized seed/scan arithmetic remains a numerical counter, not native serving proof.
Oracle: after serving only, independent full-source float64 error intervals isolate exact Fraction oracle candidates; every actual refinement is independently checked with Fraction.
Accounting:4096seed square roots/query, extra norm-upper field, all raw-refinement/metadata/audit bytes, signed corrections, upper-threshold inflation and actual final rankings; no oracle-work subtraction.
Resource guard:<=1M128D, block4096, <=65536refined or oracle-ambiguous rows/query; excess rejects/incompletes honestly; NumPy arrays<=256MiB plus capped Python audit state and mapped source<=512MiB.
Gate: meanadditional<=2048,p99<=8192,all correctness counters0,exact final parity; any failure prevents hard transfer and does not retire exact seeding.
Artifacts: upper_seed_counter.py,b8-cost.json; root runs with existing E36 receipt; no build/install/paid call or performance claim.
