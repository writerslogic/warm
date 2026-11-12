<!-- Copyright 2024-2026 WritersLogic Contributors -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Vector update format 1

`logged` is a checkpoint with a committed insert, replace, and delete batch.
`consolidated` is the same live state after generation publication and log rotation.
The corpus, fixed identity, byte comparison, and writable-open checks are in
`qgraph::update::tests::vector_format_golden_and_drift_gate`.
Existing fixtures must not be changed; new formats require a new directory.

These fixtures hold f32 rows (`HMSVRW01`) and must stay readable. `../vector-updates-v1-u8-rows` holds what the current writer produces for byte-valued rows (`HMSVRW02`).
