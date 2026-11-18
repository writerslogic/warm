// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

pub fn chunk_document(input: &DocumentInput) -> Result<Vec<DocumentChunk>> {
    Ok(chunk_spans(input)?
        .into_iter()
        .enumerate()
        .map(|(index, (start_byte, end_byte))| DocumentChunk {
            id: chunk_id(&input.id, index),
            text: input.text[start_byte as usize..end_byte as usize].into(),
            start_byte,
            end_byte,
        })
        .collect())
}

pub(super) fn chunk_id(document: &str, index: usize) -> String {
    let mut length = [0u8; 20];
    let mut ordinal = [0u8; 20];
    let (length, ordinal) = (
        decimal(document.len(), &mut length),
        decimal(index, &mut ordinal),
    );
    // Sized exactly: the id is stored as is, and a grown buffer would be reallocated to fit.
    let mut id = String::with_capacity(
        CHUNK_PREFIX.len() + length.len() + document.len() + ordinal.len() + 2,
    );
    for part in [CHUNK_PREFIX, length, ":", document, ":", ordinal] {
        id.push_str(part);
    }
    id
}

/// Whether `id` is [`chunk_id`]`(document, index)`, without building it.
pub(super) fn is_chunk_id(id: &str, document: &str, index: usize) -> bool {
    let mut length = [0u8; 20];
    let mut ordinal = [0u8; 20];
    let (length, ordinal) = (
        decimal(document.len(), &mut length),
        decimal(index, &mut ordinal),
    );
    let mut rest = id;
    for part in [CHUNK_PREFIX, length, ":", document, ":", ordinal] {
        match rest.strip_prefix(part) {
            Some(after) => rest = after,
            None => return false,
        }
    }
    rest.is_empty()
}

/// `value` in decimal, written to the end of `buffer`.
pub(super) fn decimal(mut value: usize, buffer: &mut [u8; 20]) -> &str {
    let mut at = buffer.len();
    loop {
        at -= 1;
        buffer[at] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    std::str::from_utf8(&buffer[at..]).expect("ASCII digits")
}

/// A JSON sink that counts bytes and fails once they pass `limit`, so a size check allocates
/// nothing.
pub(super) struct SizeLimit {
    pub(super) bytes: usize,
    pub(super) limit: usize,
}

impl std::io::Write for SizeLimit {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let next = self
            .bytes
            .checked_add(bytes.len())
            .filter(|&n| n <= self.limit);
        match next {
            Some(next) => {
                self.bytes = next;
                Ok(bytes.len())
            }
            None => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "document batch size limit exceeded",
            )),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The length of `value` as JSON, or an error once it passes `limit` bytes.
pub(crate) fn serialized_size(value: &impl serde::Serialize, limit: usize) -> Result<usize> {
    let mut sink = SizeLimit { bytes: 0, limit };
    serde_json::to_writer(&mut sink, value)?;
    Ok(sink.bytes)
}

/// The byte span of every chunk [`chunk_document`] cuts `input` into, with the same checks and
/// errors, without copying any text.
pub(crate) fn chunk_spans_with(
    input: &DocumentInput,
    options: &IngestOptions,
) -> Result<Vec<(u32, u32)>> {
    if !options.whole_document {
        return chunk_spans(input);
    }
    ensure!(
        options.encode == EncodeMode::Lexical && options.positions,
        "whole-document ingest requires lexical encoding and positions"
    );
    validate_document_source(input)?;
    ensure!(
        input.text.chars().any(|c| !c.is_whitespace()),
        "document must contain text"
    );
    Ok(vec![(0, input.text.len() as u32)])
}

pub(super) fn validate_document_source(input: &DocumentInput) -> Result<()> {
    ensure!(
        !input.id.is_empty() && input.id.len() <= 4096 && !input.id.contains('\0'),
        "document id must contain 1..=4096 bytes"
    );
    ensure!(
        input.text.len() <= MAX_DOCUMENT_BYTES,
        "document exceeds 8 MiB"
    );
    Ok(())
}

pub(crate) fn chunk_spans(input: &DocumentInput) -> Result<Vec<(u32, u32)>> {
    validate_document_source(input)?;
    let size = input.chunk_words.unwrap_or(256) as usize;
    let overlap = input
        .overlap_words
        .unwrap_or(32.min(size.saturating_sub(1)) as u32) as usize;
    ensure!(
        (1..=4096).contains(&size) && overlap < size,
        "chunkWords must be 1..=4096 and overlapWords smaller than chunkWords"
    );
    if input.text.is_ascii() {
        // `char::is_whitespace` on ASCII is exactly 0x09..=0x0D and the space.
        let space = |b: u8| matches!(b, b' ' | b'\t'..=b'\r');
        let bytes = input.text.as_bytes();
        // A text of at most `size` words is one chunk, its first word's start to its last
        // word's end: count the word starts without a branch per byte and skip collecting
        // every word's span.
        let count = bytes.first().is_some_and(|&b| !space(b)) as usize
            + bytes
                .iter()
                .zip(bytes.get(1..).unwrap_or_default())
                .map(|(&a, &b)| (space(a) & !space(b)) as usize)
                .sum::<usize>();
        if (1..=size).contains(&count) {
            let start_byte = bytes.iter().position(|&b| !space(b)).expect("a word");
            let end_byte = 1 + bytes.iter().rposition(|&b| !space(b)).expect("a word");
            ensure!(
                end_byte - start_byte <= MAX_CHUNK_BYTES,
                "chunk exceeds 64 KiB; reduce chunkWords or split long tokens"
            );
            return Ok(vec![(start_byte as u32, end_byte as u32)]);
        }
    }
    let mut words = Vec::with_capacity(input.text.len() / 6);
    let mut start = None;
    let mut boundary = |i: usize, whitespace: bool| {
        if whitespace {
            if let Some(s) = start.take() {
                words.push((s, i));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    };
    if input.text.is_ascii() {
        // `char::is_whitespace` on ASCII is exactly 0x09..=0x0D and the space.
        for (i, b) in input.text.bytes().enumerate() {
            boundary(i, matches!(b, b' ' | b'\t'..=b'\r'));
        }
    } else {
        for (i, c) in input.text.char_indices() {
            boundary(i, c.is_whitespace());
        }
    }
    if let Some(s) = start {
        words.push((s, input.text.len()));
    }
    ensure!(!words.is_empty(), "document must contain text");
    let mut chunks = Vec::new();
    let mut word = 0;
    while word < words.len() {
        ensure!(
            chunks.len() < 4096,
            "document exceeds 4096 chunks; increase chunkWords"
        );
        let end_word = (word + size).min(words.len());
        let (start_byte, end_byte) = (words[word].0, words[end_word - 1].1);
        ensure!(
            end_byte - start_byte <= MAX_CHUNK_BYTES,
            "chunk exceeds 64 KiB; reduce chunkWords or split long tokens"
        );
        chunks.push((start_byte as u32, end_byte as u32));
        if end_word == words.len() {
            break;
        }
        word += size - overlap;
    }
    Ok(chunks)
}

/// Validates `values` (the store's dimensions, finite, not all zero) and returns them
/// l2-normalised in `f64`. Accepts the client's `f64` values and a stored [`Embedding`]'s
/// `f32` values, which widen exactly.
pub(crate) fn normalize_embedding<T: Copy + Into<f64>>(
    values: &[T],
    dimensions: usize,
) -> Result<Vec<f64>> {
    ensure!(
        values.len() == dimensions && values.iter().all(|&x| Into::<f64>::into(x).is_finite()),
        "embedding dimensions must match the store and values must be finite"
    );
    let scale = values
        .iter()
        .map(|&x| Into::<f64>::into(x).abs())
        .fold(0.0_f64, f64::max);
    ensure!(scale > 0.0, "zero embeddings are not supported");
    let norm = values
        .iter()
        .map(|&x| (Into::<f64>::into(x) / scale).powi(2))
        .sum::<f64>()
        .sqrt();
    Ok(values
        .iter()
        .map(|&x| (Into::<f64>::into(x) / scale) / norm)
        .collect())
}
