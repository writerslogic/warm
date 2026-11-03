# Derive the representation from what may change

The next mechanism screen applies control theory to the memory contract. Instead
of asking which scores can be compressed today, it asks which distinctions any
allowed future mutation can make observable. This is a different design route
from another nearest-neighbor index. The algebra is established observability
theory; applying it here does not establish an invention.

Let the association state be `x`, required linear score contrasts be `C x`, and
an allowed replacement or deletion be `x' = D x + b`. The new contribution `b`
must be supplied by the caller; a deletion supplies zero. Seek an encoding
`z = E x`, a readout `R`, and an update `U_D` satisfying the exact identities

```
C = R E
E D = U_D E
z' = U_D z + E b
```

The compiler starts from the row space of C and repeatedly adds each existing row
multiplied by each allowed D. At closure, all required observations and future
mutations are representable. Its exact rational basis supplies E; row-space
coordinates supply R and every U. Any other linear encoding satisfying those
identities must contain that closed row space. This establishes minimal rank for
these linear observations and mutation operators, not minimum bytes or a theorem
about every nonlinear encoding that preserves only a winning label.

This construction is closely related to unobservable invariant subspaces and
observability reduction in switched systems. Relevant primary sources include
[Eising and Trentelman](https://www.math.rug.nl/~trentelman/psfiles/Info_properties_noisy.pdf)
and [Hespanha et al.](https://web.ece.ucsb.edu/~hespanha/published/cdc02-observability.pdf).
The closure computation and its identity proof are controls to build on, not the
claimed scientific contribution.

## Result of the exact screen

For a two-row, three-output association table, all current score contrasts need
four coordinates. If forgetting or replacement acts on an entire contribution
row, four remain sufficient: a common offset across that row's outputs is never
observable, even after later row changes. If individual cells can be deleted,
closure expands to all six coordinates. A discarded offset can become a real
answer difference, as the earlier [counterexample](MUTATION.md) showed.

| Table | Whole-row changes: required rank | Individual-cell changes: required rank |
|---|---:|---:|
| 2 × 2 | 2 of 4 | 4 of 4 |
| 2 × 3 | 4 of 6 | 6 of 6 |
| 4 × 8 | 28 of 32 | 32 of 32 |

Both contracts passed exact matrix identities, all 729 initial tables with weights
0/1/2, 30,618 cue readouts, 17,496 single-mutation checks and 2,000 sequential
replacement/deletion checks in total. Every compiled update matched fresh encoding
of the changed full state. Three boundary/counterexample regression tests passed.
These are exhaustive checks of the declared small input domain plus finite
sequence checks; the matrix identities provide the general linear argument.

This finds a valid restricted case that the earlier individual-cell counterexample
did not rule out. It also rejects a linear compression promise for that broader
mutation contract. A contribution row here is a vector of answer scores, not a
document, source citation or arbitrary fact. Changing the product's forgetting
semantics to fit the smaller encoding would not be an advance.

## What this does and does not justify next

The useful research object is a compiler that accepts query and mutation semantics
and emits a representation with checkable update identities. The current dense
toy is not that product: its 2 × 3 row case stores 24 encoding coefficients and
32 mutation coefficients in addition to four state coordinates. Rational widths,
source/provenance state and all retained operators cost space. No memory or speed
advantage is inferred from the rank table.

Before promotion, identify a real memory workload whose observation and mutation
structure permits a sparse, implicit representation; include provenance in the
contract and compare with an indexed exact store and incremental materialized
views. The candidate must supply a concrete capability or measured cost advantage
beyond implementing known observability reduction. If added provenance or required
updates make closure full rank, retain that negative result instead of omitting
the operation. No new runtime or tuning sweep is licensed by this toy alone.

Source and pre-run plan: `benchmarks/research/mutation_quotient/`.
Frozen output: `/Volumes/A/.hms-bench-cache/warm-mutation-quotient-20261103-v1`.
Freeze SHA256: `9314814d4fb631a088c20c05feb2e8ec7211ecc5d97372e7ec73247a51d9e00d`.
Receipts and logs are copied beside this document.
