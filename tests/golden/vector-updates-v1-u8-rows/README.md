# Vector update format v1, byte-valued rows

What the current writer produces for the corpus of `../vector-updates-v1` when every
coordinate is an integer in 0..255: rows are stored one byte per coordinate (`HMSVRW02`).
`../vector-updates-v1` holds the f32 rows (`HMSVRW01`) and must stay readable.

The byte comparison and open checks are in
`qgraph::update::tests::vector_format_golden_and_drift_gate`.
Existing fixtures must not be changed; new formats require a new directory.
