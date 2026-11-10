# H-B-7 native arithmetic gate
Limitation: E24 scalar endpoints do not validate native arithmetic, metadata or floating-environment restoration.
Mechanism: strict C reordered interval epilogue; checked uint128 source/error squares; exact integer rounding of scale/norm/radius/threshold metadata.
Domain: mathematical dimensions1..128, finite binary32 coordinates zero or magnitude2^-16..2^16; unsupported finite inputs within ABI caps request Fraction fallback; ABI dimensions1..4096 and kth rows1..64, outside caps invalid.
Prediction: native intervals enclose independent Fraction scores, exact integer candidate ordering agrees, and caller rounding/exception state survives success and error.
Fixtures: fixed1/2/99/100/101/127/128/129D; allocated4095/4096/4097D and0/1/2/63/64/65row boundaries; endpoint neighbors,uint128 extrema,signed dots,ties,kth thresholds,invalid inputs; no sweep.
Gate: zero failed checks, nonzero strict native pruning, unsupported-finite fallback and all four caller rounding modes exercised; no native throughput claim.
Budget:180seconds for compiler identity, one shared-library build, disassembly and fixed checks; timeout/failure retains full logs, hashes and a failure receipt.
Flags:clang -std=c11 -O2 -Wall -Wextra -Werror -fno-fast-math -ffp-model=strict -ffp-contract=off -fPIC plus platform shared-library flag; no install.
Run:uv run --offline --no-project --python /Volumes/A/.hms-lb/bigann/venv/bin/python benchmarks/results/mission/B/run_native_interval.py --output-dir /Volumes/A/.hms-wt-mission-integrated/benchmarks/results/mission/root/b7-native
Scope: standalone correctness only; engine, VNNI performance, seed-cost gate, official score audit and complete resource validation remain unchanged and open.
