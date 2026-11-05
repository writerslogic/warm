# K-mer encoder for DNA and RNA

`KmerEncoder` turns a nucleotide sequence into an `EntangledHVec`, the engine's sparse
hypervector, so sequences can be compared with the same overlap measures as everything else. It
is experimental: build with `--features experimental`. It has been evaluated only on synthetic
random references; see
[the evaluation](../research/KMER-ENCODER-EVAL-2026-11-03.md) for what was and was not measured.

```rust
use writerslogic_warm::core::kmer::{KmerConfig, KmerEncoder};

let encoder = KmerEncoder::new(KmerConfig { k: 15, dim: 1 << 20, ..KmerConfig::default() })?;
let reference = encoder.encode(b"ACGTACGT...")?;
let read = encoder.encode(b"acgtacgu...")?; // case-insensitive; U is read as T
let score = read.containment_similarity(&reference); // fraction of the read's bits in the reference
```

## What it computes

Each window of `k` bases is one k-mer. With `canonical: true` (the default) a k-mer is replaced
by the smaller of itself and its reverse complement, so a sequence and its reverse complement
encode to the same vector. Each k-mer is hashed to `bits_per_kmer` active indices in
`[0, dim)`; the sequence vector is the union of those. That makes the vector a Bloom filter over
the sequence's k-mers, not a majority-vote bundle (a majority vote over thousands of k-mers is
empty). Use `containment_similarity(read, reference)` to ask how much of a read is in a
reference, `similarity` (Jaccard) for sequences of similar length.

## Parameters

| Field | Range | Default | Notes |
|---|---|---|---|
| `k` | 4 to 32 | 15 | Longer k is more specific and loses more windows to each mismatch. |
| `dim` | 64 to 2^26 | 2^20 | Keep `windows * bits_per_kmer` well below `dim`, or the vector saturates and every pair scores high. |
| `bits_per_kmer` | 1 to 16 | 2 | |
| `canonical` | | `true` | Turn off only if strand is meaningful. |
| `seed` | any | 0 | Vectors from different seeds are unrelated. |
| `max_len` | 1 to 2^30 | 2^24 | Longer input is rejected before any work. |

## Symbols and errors

- `A C G T U`, either case, are bases.
- IUPAC ambiguity codes (`R Y S W K M B D H V N`, either case) are accepted; every window that
  contains one is skipped. `encode_with_stats` reports encoded and skipped window counts.
- Any other byte (whitespace, `-`, digits) is `KmerError::InvalidSymbol` with its offset. Strip
  FASTA headers and line breaks first.
- A sequence shorter than `k`, or with no unambiguous window, encodes to the empty vector.
- Bad configuration and over-long input are `KmerError`s, never panics. `KmerError` converts into
  `WarmError`.

## Limits

The evaluation covers 5,000 bp random references, 150 bp reads, `k` in 9, 11, 15, 21 and the
dimensions listed there. Nothing beyond that range, and no real genome, has been measured.
