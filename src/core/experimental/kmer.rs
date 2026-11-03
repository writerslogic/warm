// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! K-mer sparse encoding for DNA and RNA sequences.
//!
//! [`KmerEncoder`] maps a nucleotide sequence to an [`EntangledHVec`], the engine's sparse
//! hypervector, so sequences can be compared with the engine's own overlap measures
//! ([`EntangledHVec::similarity`], [`EntangledHVec::containment_similarity`],
//! [`EntangledHVec::hamming`]).
//!
//! # Construction
//!
//! 1. Every window of `k` consecutive bases is a k-mer. Bases are `A C G T` (case-insensitive,
//!    `U` is read as `T`), packed two bits per base, so `k` is limited to 32.
//! 2. With `canonical` set, a window is replaced by the numerically smaller of itself and its
//!    reverse complement, so a sequence and its reverse complement encode identically.
//! 3. A k-mer code is hashed to `bits_per_kmer` active indices in `[0, dim)`
//!    (`hash_u64(hash_u64(hash_u64(seed, k), code), j) % dim`). This is a pure function of
//!    `(seed, k, code, dim, bits_per_kmer)`: the same k-mer has the same indices in every
//!    sequence, in every process.
//! 4. The sequence vector is the set union of its k-mer vectors, exactly
//!    [`EntangledHVec::bundle_bloom`] of the per-k-mer vectors. Majority-vote
//!    [`EntangledHVec::bundle`] is not used: it keeps only indices present in at least half of
//!    the inputs, and thousands of k-mer vectors share almost no index, so it returns the
//!    empty set.
//!
//! # What this is
//!
//! A sequence vector is a Bloom filter over the sequence's k-mer set. Containment of a read in a
//! reference, `read.containment_similarity(&reference)`, is the fraction of the read's active
//! indices present in the reference's; hash collisions can only raise it. Retrieval quality is
//! therefore bounded above by exact k-mer matching on the same `k`, and the filter gets denser
//! (and less selective) as `dim` shrinks or the sequence grows: choose `dim` so that
//! `windows * bits_per_kmer` stays well below `dim`. The sequence vector is not sparse in the
//! `dim / 256` sense of [`EntangledHVec::new_deterministic`]; its active count is up to
//! `windows * bits_per_kmer`.
//!
//! # Symbols
//!
//! - `A C G T U` (either case) are bases.
//! - The IUPAC ambiguity codes `R Y S W K M B D H V N` (either case) are accepted but are not
//!   bases: every window that contains one is skipped, and no window spans
//!   an ambiguity code. The k-mers on either side of it are still encoded.
//! - Any other byte, including whitespace, line breaks and `-`, is an error that reports its
//!   offset. Strip FASTA headers and line breaks before encoding.
//!
//! # Limits
//!
//! Sequences longer than [`KmerConfig::max_len`] are rejected before any work. Working memory
//! is one bitset of `dim` bits plus the output (at most `min(dim, windows * bits_per_kmer)`
//! `u32` indices); it does not grow with the sequence beyond that.

use crate::core::entangled::{hash_u64, EntangledHVec};
use crate::core::error::WarmError;
use thiserror::Error;

/// Smallest accepted `k`.
pub const MIN_K: usize = 4;
/// Largest accepted `k`: a k-mer is packed two bits per base into a `u64`.
pub const MAX_K: usize = 32;
/// Smallest accepted vector dimension.
pub const MIN_DIM: usize = 64;
/// Largest accepted vector dimension; bounds the working bitset to 8 MiB.
pub const MAX_DIM: usize = 1 << 26;
/// Largest accepted number of active indices per k-mer.
pub const MAX_BITS_PER_KMER: usize = 16;
/// Largest accepted [`KmerConfig::max_len`].
pub const MAX_SEQUENCE_LEN: usize = 1 << 30;
/// Default [`KmerConfig::max_len`]: 16 Mi bases.
pub const DEFAULT_MAX_LEN: usize = 1 << 24;

/// A failure to configure the encoder or to encode a sequence.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KmerError {
    #[error("invalid k-mer encoder configuration: {0}")]
    InvalidConfig(String),
    /// A byte that is neither a base nor an IUPAC ambiguity code.
    #[error("invalid sequence symbol 0x{byte:02x} at offset {offset}")]
    InvalidSymbol { offset: usize, byte: u8 },
    #[error("sequence length {len} exceeds the limit of {max}")]
    SequenceTooLong { len: usize, max: usize },
}

impl From<KmerError> for WarmError {
    fn from(error: KmerError) -> Self {
        match error {
            KmerError::SequenceTooLong { .. } => WarmError::CapacityExceeded {
                limit: error.to_string(),
            },
            other => WarmError::InvalidParam {
                details: other.to_string(),
            },
        }
    }
}

/// Encoder parameters. Two encoders with equal configs produce identical vectors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KmerConfig {
    /// K-mer length, in `MIN_K..=MAX_K`.
    pub k: usize,
    /// Vector dimension, in `MIN_DIM..=MAX_DIM`.
    pub dim: usize,
    /// Active indices per k-mer, in `1..=MAX_BITS_PER_KMER` (hash collisions inside one k-mer
    /// can leave fewer).
    pub bits_per_kmer: usize,
    /// Encode `min(kmer, reverse_complement(kmer))` so strand does not matter.
    pub canonical: bool,
    /// Hash seed; vectors from different seeds are unrelated.
    pub seed: u64,
    /// Longest accepted sequence in bases, in `1..=MAX_SEQUENCE_LEN`.
    pub max_len: usize,
}

impl Default for KmerConfig {
    fn default() -> Self {
        Self {
            k: 15,
            dim: 1 << 20,
            bits_per_kmer: 2,
            canonical: true,
            seed: 0,
            max_len: DEFAULT_MAX_LEN,
        }
    }
}

impl KmerConfig {
    fn validate(&self) -> Result<(), KmerError> {
        let bad = |msg: String| Err(KmerError::InvalidConfig(msg));
        if !(MIN_K..=MAX_K).contains(&self.k) {
            return bad(format!("k = {} is outside {MIN_K}..={MAX_K}", self.k));
        }
        if !(MIN_DIM..=MAX_DIM).contains(&self.dim) {
            return bad(format!(
                "dim = {} is outside {MIN_DIM}..={MAX_DIM}",
                self.dim
            ));
        }
        if !(1..=MAX_BITS_PER_KMER).contains(&self.bits_per_kmer) {
            return bad(format!(
                "bits_per_kmer = {} is outside 1..={MAX_BITS_PER_KMER}",
                self.bits_per_kmer
            ));
        }
        if !(1..=MAX_SEQUENCE_LEN).contains(&self.max_len) {
            return bad(format!(
                "max_len = {} is outside 1..={MAX_SEQUENCE_LEN}",
                self.max_len
            ));
        }
        Ok(())
    }
}

/// A sequence vector with the window accounting that produced it.
#[derive(Debug, Clone)]
pub struct KmerEncoding {
    pub vector: EntangledHVec,
    /// Windows that contained only bases and were encoded.
    pub windows_encoded: usize,
    /// Windows skipped because they contained an ambiguity code.
    pub windows_skipped: usize,
}

/// Maps nucleotide sequences to sparse hypervectors. See the [module documentation](self).
#[derive(Debug, Clone)]
pub struct KmerEncoder {
    config: KmerConfig,
    /// `hash_u64(seed, k)`: k-mers of different lengths never share indices by construction.
    kmer_seed: u64,
    mask: u64,
}

enum Symbol {
    Base(u8),
    Ambiguous,
}

fn classify(byte: u8) -> Option<Symbol> {
    match byte {
        b'A' | b'a' => Some(Symbol::Base(0)),
        b'C' | b'c' => Some(Symbol::Base(1)),
        b'G' | b'g' => Some(Symbol::Base(2)),
        b'T' | b't' | b'U' | b'u' => Some(Symbol::Base(3)),
        b'R' | b'Y' | b'S' | b'W' | b'K' | b'M' | b'B' | b'D' | b'H' | b'V' | b'N' | b'r'
        | b'y' | b's' | b'w' | b'k' | b'm' | b'b' | b'd' | b'h' | b'v' | b'n' => {
            Some(Symbol::Ambiguous)
        }
        _ => None,
    }
}

impl KmerEncoder {
    /// Validates `config` and builds the encoder.
    pub fn new(config: KmerConfig) -> Result<Self, KmerError> {
        config.validate()?;
        let mask = if config.k == MAX_K {
            u64::MAX
        } else {
            (1u64 << (2 * config.k)) - 1
        };
        Ok(Self {
            kmer_seed: hash_u64(config.seed, config.k as u64),
            mask,
            config,
        })
    }

    pub fn config(&self) -> &KmerConfig {
        &self.config
    }

    /// The sparse vector of one k-mer code: up to `bits_per_kmer` sorted indices in `[0, dim)`.
    pub fn kmer_sdr(&self, code: u64) -> EntangledHVec {
        let mut indices: Vec<u32> = (0..self.config.bits_per_kmer)
            .map(|j| self.kmer_index(code, j))
            .collect();
        indices.sort_unstable();
        indices.dedup();
        EntangledHVec::from_indices(indices, self.config.dim)
    }

    /// The vector of `sequence`: the union of its k-mer vectors. A sequence with no encodable
    /// window (shorter than `k`, or every window ambiguous) encodes to the empty vector of
    /// dimension `dim`.
    pub fn encode(&self, sequence: &[u8]) -> Result<EntangledHVec, KmerError> {
        Ok(self.encode_with_stats(sequence)?.vector)
    }

    /// [`Self::encode`] plus how many windows were encoded and skipped.
    pub fn encode_with_stats(&self, sequence: &[u8]) -> Result<KmerEncoding, KmerError> {
        let dim = self.config.dim;
        let mut bits = vec![0u64; dim.div_ceil(64)];
        let windows_encoded = self.scan(sequence, |code| {
            for j in 0..self.config.bits_per_kmer {
                let index = self.kmer_index(code, j) as usize;
                bits[index / 64] |= 1u64 << (index % 64);
            }
        })?;
        let total = sequence.len().saturating_sub(self.config.k - 1);
        let active: usize = bits.iter().map(|w| w.count_ones() as usize).sum();
        let mut indices = Vec::with_capacity(active);
        for (word_index, &word) in bits.iter().enumerate() {
            let mut word = word;
            while word != 0 {
                indices.push((word_index * 64 + word.trailing_zeros() as usize) as u32);
                word &= word - 1;
            }
        }
        Ok(KmerEncoding {
            vector: EntangledHVec::from_indices(indices, dim),
            windows_encoded,
            windows_skipped: total - windows_encoded,
        })
    }

    /// The k-mer code of every encodable window, in sequence order and with repeats, after
    /// canonicalization when configured. For exact k-mer baselines that must share this
    /// encoder's alphabet, ambiguity and strand rules.
    pub fn kmer_codes(&self, sequence: &[u8]) -> Result<Vec<u64>, KmerError> {
        let mut codes = Vec::new();
        self.scan(sequence, |code| codes.push(code))?;
        Ok(codes)
    }

    fn kmer_index(&self, code: u64, j: usize) -> u32 {
        (hash_u64(hash_u64(self.kmer_seed, code), j as u64) % self.config.dim as u64) as u32
    }

    /// Calls `emit` for each encodable window and returns how many there were. The whole
    /// sequence is validated even when no window is encodable.
    fn scan(&self, sequence: &[u8], mut emit: impl FnMut(u64)) -> Result<usize, KmerError> {
        let max = self.config.max_len;
        if sequence.len() > max {
            return Err(KmerError::SequenceTooLong {
                len: sequence.len(),
                max,
            });
        }
        let k = self.config.k;
        let rc_shift = 2 * (k - 1);
        let (mut forward, mut reverse, mut run, mut encoded) = (0u64, 0u64, 0usize, 0usize);
        for (offset, &byte) in sequence.iter().enumerate() {
            match classify(byte) {
                None => return Err(KmerError::InvalidSymbol { offset, byte }),
                Some(Symbol::Ambiguous) => {
                    forward = 0;
                    reverse = 0;
                    run = 0;
                }
                Some(Symbol::Base(base)) => {
                    let base = u64::from(base);
                    forward = ((forward << 2) | base) & self.mask;
                    reverse = (reverse >> 2) | ((3 - base) << rc_shift);
                    run = (run + 1).min(k);
                    if run == k {
                        emit(if self.config.canonical {
                            forward.min(reverse)
                        } else {
                            forward
                        });
                        encoded += 1;
                    }
                }
            }
        }
        Ok(encoded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg_sequence(seed: u64, len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| b"ACGT"[(hash_u64(seed, i as u64) % 4) as usize])
            .collect()
    }

    fn reverse_complement(sequence: &[u8]) -> Vec<u8> {
        sequence
            .iter()
            .rev()
            .map(|&b| match b {
                b'A' => b'T',
                b'C' => b'G',
                b'G' => b'C',
                b'T' => b'A',
                other => other,
            })
            .collect()
    }

    fn encoder(k: usize, canonical: bool) -> KmerEncoder {
        KmerEncoder::new(KmerConfig {
            k,
            dim: 1 << 16,
            canonical,
            ..KmerConfig::default()
        })
        .unwrap()
    }

    #[test]
    fn deterministic_across_instances_and_case() {
        let sequence = lcg_sequence(1, 300);
        let a = encoder(11, true).encode(&sequence).unwrap();
        let b = encoder(11, true).encode(&sequence).unwrap();
        assert_eq!(a.indices(), b.indices());
        assert!(!a.indices().is_empty());

        let lower: Vec<u8> = sequence.to_ascii_lowercase();
        assert_eq!(
            encoder(11, true).encode(&lower).unwrap().indices(),
            a.indices()
        );

        let rna: Vec<u8> = sequence
            .iter()
            .map(|&b| if b == b'T' { b'U' } else { b })
            .collect();
        assert_eq!(
            encoder(11, true).encode(&rna).unwrap().indices(),
            a.indices()
        );

        let other_seed = KmerEncoder::new(KmerConfig {
            k: 11,
            dim: 1 << 16,
            seed: 7,
            ..KmerConfig::default()
        })
        .unwrap();
        assert!(other_seed.encode(&sequence).unwrap().similarity(&a) < 0.2);
    }

    #[test]
    fn single_window_equals_its_kmer_vector() {
        let encoder = KmerEncoder::new(KmerConfig {
            k: 4,
            dim: 1 << 12,
            bits_per_kmer: 3,
            canonical: false,
            ..KmerConfig::default()
        })
        .unwrap();
        let sdr = encoder.encode(b"ACGT").unwrap();
        assert_eq!(sdr.indices(), encoder.kmer_sdr(0b00_01_10_11).indices());
        assert!(sdr.indices().len() <= 3 && !sdr.indices().is_empty());
    }

    #[test]
    fn canonical_is_strand_invariant() {
        for k in [4, 11, 31, 32] {
            let sequence = lcg_sequence(2, 200);
            let rc = reverse_complement(&sequence);
            let canonical = encoder(k, true);
            assert_eq!(
                canonical.encode(&sequence).unwrap().indices(),
                canonical.encode(&rc).unwrap().indices(),
                "k = {k}"
            );
            if k >= 8 {
                // At k = 4 a 200 bp sequence already covers half of all 256 k-mers on each strand.
                let plain = encoder(k, false);
                let (f, r) = (plain.encode(&sequence).unwrap(), plain.encode(&rc).unwrap());
                assert!(
                    f.similarity(&r) < 0.2,
                    "k = {k}: strands differ without canonical"
                );
            }
        }
    }

    #[test]
    fn ambiguity_codes_skip_windows_and_never_bridge() {
        let encoder = encoder(4, false);
        let encoding = encoder.encode_with_stats(b"ACGTNACGT").unwrap();
        // Windows: ACGT, CGTN, GTNA, TNAC, NACG, ACGT. Four contain the N.
        assert_eq!(encoding.windows_encoded, 2);
        assert_eq!(encoding.windows_skipped, 4);
        assert_eq!(
            encoding.vector.indices(),
            encoder.encode(b"ACGT").unwrap().indices()
        );
        for iupac in "RYSWKMBDHVNrywskmbdhvn".bytes() {
            let mut sequence = b"ACGTA".to_vec();
            sequence.push(iupac);
            sequence.extend_from_slice(b"CGTAC");
            let stats = encoder.encode_with_stats(&sequence).unwrap();
            assert_eq!(stats.windows_encoded, 4, "{}", iupac as char);
            assert_eq!(stats.windows_skipped, 4, "{}", iupac as char);
        }
        let all_ambiguous = encoder.encode_with_stats(b"NNNNNNNN").unwrap();
        assert_eq!(all_ambiguous.windows_encoded, 0);
        assert!(all_ambiguous.vector.indices().is_empty());
    }

    #[test]
    fn invalid_symbols_are_errors_with_offsets() {
        let encoder = encoder(4, true);
        for (sequence, offset, byte) in [
            (&b"ACGT ACGT"[..], 4usize, b' '),
            (b"ACGT\nACGT", 4, b'\n'),
            (b"ACGT-ACGT", 4, b'-'),
            (b"ACGTX", 4, b'X'),
            (b"NN1", 2, b'1'),
            (b"ACGT\xff", 4, 0xff),
        ] {
            assert_eq!(
                encoder.encode(sequence).unwrap_err(),
                KmerError::InvalidSymbol { offset, byte }
            );
        }
    }

    #[test]
    fn boundary_lengths_around_k() {
        let k = 8;
        let encoder = encoder(k, true);
        let sequence = lcg_sequence(3, k + 1);

        let short = encoder.encode_with_stats(&sequence[..k - 1]).unwrap();
        assert_eq!((short.windows_encoded, short.windows_skipped), (0, 0));
        assert!(short.vector.indices().is_empty());
        assert_eq!(short.vector.dim, 1 << 16);

        let exact = encoder.encode_with_stats(&sequence[..k]).unwrap();
        assert_eq!((exact.windows_encoded, exact.windows_skipped), (1, 0));
        let bits = encoder.config().bits_per_kmer;
        assert!((1..=bits).contains(&exact.vector.indices().len()));

        let longer = encoder.encode_with_stats(&sequence).unwrap();
        assert_eq!((longer.windows_encoded, longer.windows_skipped), (2, 0));
        assert!(longer.vector.indices().len() >= exact.vector.indices().len());

        let empty = encoder.encode_with_stats(b"").unwrap();
        assert_eq!(empty.windows_encoded, 0);
        assert!(empty.vector.indices().is_empty());
    }

    #[test]
    fn length_limit_boundaries() {
        let limit = 64;
        let encoder = KmerEncoder::new(KmerConfig {
            k: 8,
            dim: 1 << 12,
            max_len: limit,
            ..KmerConfig::default()
        })
        .unwrap();
        let sequence = lcg_sequence(4, limit + 1);
        assert!(encoder.encode(&sequence[..limit - 1]).is_ok());
        assert!(encoder.encode(&sequence[..limit]).is_ok());
        assert_eq!(
            encoder.encode(&sequence).unwrap_err(),
            KmerError::SequenceTooLong {
                len: limit + 1,
                max: limit
            }
        );
        // Over-limit input is rejected before its symbols are examined.
        assert!(matches!(
            encoder.encode(&vec![b'!'; limit + 1]),
            Err(KmerError::SequenceTooLong { .. })
        ));
    }

    #[test]
    fn config_range_boundaries() {
        let with = |f: fn(&mut KmerConfig)| {
            let mut config = KmerConfig::default();
            f(&mut config);
            KmerEncoder::new(config).map(|_| ())
        };
        assert!(with(|c| c.k = MIN_K - 1).is_err());
        assert!(with(|c| c.k = 0).is_err());
        assert!(with(|c| c.k = MIN_K).is_ok());
        assert!(with(|c| c.k = MAX_K).is_ok());
        assert!(with(|c| c.k = MAX_K + 1).is_err());
        assert!(with(|c| c.dim = MIN_DIM - 1).is_err());
        assert!(with(|c| c.dim = MIN_DIM).is_ok());
        assert!(with(|c| c.dim = MAX_DIM).is_ok());
        assert!(with(|c| c.dim = MAX_DIM + 1).is_err());
        assert!(with(|c| c.bits_per_kmer = 0).is_err());
        assert!(with(|c| c.bits_per_kmer = MAX_BITS_PER_KMER).is_ok());
        assert!(with(|c| c.bits_per_kmer = MAX_BITS_PER_KMER + 1).is_err());
        assert!(with(|c| c.max_len = 0).is_err());
        assert!(with(|c| c.max_len = MAX_SEQUENCE_LEN).is_ok());
        assert!(with(|c| c.max_len = MAX_SEQUENCE_LEN + 1).is_err());
    }

    #[test]
    fn sequence_vector_is_the_union_of_kmer_vectors() {
        let encoder = encoder(9, true);
        let sequence = lcg_sequence(5, 400);
        let per_kmer: Vec<EntangledHVec> = encoder
            .kmer_codes(&sequence)
            .unwrap()
            .into_iter()
            .map(|code| encoder.kmer_sdr(code))
            .collect();
        let bundled = EntangledHVec::bundle_bloom(&per_kmer);
        assert_eq!(
            encoder.encode(&sequence).unwrap().indices(),
            bundled.indices()
        );
    }

    #[test]
    fn noisy_read_scores_its_source_above_an_unrelated_reference() {
        let encoder = encoder(11, true);
        let source = lcg_sequence(6, 2000);
        let unrelated = lcg_sequence(7, 2000);
        let mut read = source[500..700].to_vec();
        for i in (0..read.len()).step_by(20) {
            read[i] = if read[i] == b'A' { b'C' } else { b'A' };
        }
        let read = encoder.encode(&read).unwrap();
        let own = read.containment_similarity(&encoder.encode(&source).unwrap());
        let other = read.containment_similarity(&encoder.encode(&unrelated).unwrap());
        assert!(own > 0.35 && own > 3.0 * other, "own {own}, other {other}");
    }
}
