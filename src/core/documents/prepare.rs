// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// [`Interner::chunk_terms`] through the batch interner installed on this thread, or a
/// one-off one.
#[cfg(test)]
pub(super) fn chunk_terms(text: &str, title_weight: u32, stem: bool) -> (StoredTerms, usize) {
    analyze_chunk(text, title_weight, stem, None, Analyzer::Legacy)
}

/// A chunk's term table through the batch interner installed on this thread, or a one-off
/// one, with each term's word positions when `positions` is `Some(whole_document)`.
pub(super) fn analyze_chunk(
    text: &str,
    title_weight: u32,
    stem: bool,
    positions: Option<bool>,
    analyzer: Analyzer,
) -> (StoredTerms, usize) {
    ACTIVE_STEM_MEMO.with(|active| match active.borrow_mut().as_mut() {
        Some(interner) => interner.chunk_terms(text, title_weight, stem, positions, analyzer),
        None => Interner::default().chunk_terms(text, title_weight, stem, positions, analyzer),
    })
}

pub(crate) fn prepare(
    input: DocumentInput,
    dimensions: usize,
    embedding_dimensions: Option<usize>,
    encoder: TextEncoder,
) -> Result<StoredDocument> {
    prepare_with_options(
        input,
        dimensions,
        embedding_dimensions,
        None,
        encoder,
        &IngestOptions::default(),
    )
}

/// [`prepare`] with an optional lexical key per chunk (for example the chunk prefixed with
/// facts extracted from it; stored text and byte offsets stay the chunk's own) and the chunk vectors encoded now, later or never (`mode`). The
/// lexical side of the document is the same in every mode.
pub(crate) fn prepare_with_options(
    input: DocumentInput,
    dimensions: usize,
    embedding_dimensions: Option<usize>,
    keys: Option<&[String]>,
    encoder: TextEncoder,
    options: &IngestOptions,
) -> Result<StoredDocument> {
    check_mode(&input, encoder, options.encode)?;
    let spans = chunk_spans_with(&input, options)?;
    prepare_spans(
        input,
        spans,
        dimensions,
        embedding_dimensions,
        keys,
        encoder,
        options,
        Analyzer::Legacy,
    )
}

pub(super) fn check_mode(
    input: &DocumentInput,
    encoder: TextEncoder,
    mode: EncodeMode,
) -> Result<()> {
    ensure!(
        mode != EncodeMode::Deferred || !encoder.is_embedding(),
        "deferred encoding applies to text encoders; {} vectors come from embeddings",
        encoder.name()
    );
    ensure!(
        mode != EncodeMode::Deferred || input.store_text.unwrap_or(true),
        "deferred encoding needs the chunk text; set storeText or encode eagerly"
    );
    Ok(())
}

/// [`prepare_with_options`] of an input whose chunk spans ([`chunk_spans`]) are known.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_spans(
    input: DocumentInput,
    chunks: Vec<(u32, u32)>,
    dimensions: usize,
    embedding_dimensions: Option<usize>,
    keys: Option<&[String]>,
    encoder: TextEncoder,
    options: &IngestOptions,
    analyzer: Analyzer,
) -> Result<StoredDocument> {
    let mode = options.encode;
    check_mode(&input, encoder, mode)?;
    ensure!(
        keys.is_none_or(|k| k.len() == chunks.len()),
        "provide one key per chunk"
    );
    ensure!(
        input.metadata.as_ref().is_none_or(Value::is_object),
        "metadata must be a JSON object"
    );
    ensure!(
        serialized_size(&input.metadata, 64 * 1024).is_ok(),
        "metadata exceeds 64 KiB"
    );
    ensure!(
        input.source_uri.as_ref().is_none_or(|s| s.len() <= 8192),
        "sourceUri exceeds 8192 bytes"
    );
    ensure!(
        input
            .version
            .as_ref()
            .is_none_or(|s| !s.is_empty() && s.len() <= 1024),
        "version requires 1..=1024 bytes"
    );
    if let Some(embeddings) = &input.embeddings {
        ensure!(
            embedding_dimensions.is_some() && embeddings.len() == chunks.len(),
            "provide one embedding per chunk and configure the store embedding space"
        );
        ensure!(
            embeddings.iter().map(Vec::len).sum::<usize>() <= 1_048_576,
            "document exceeds 1048576 embedding values; split the document"
        );
    }
    let mut stored = Vec::with_capacity(chunks.len());
    let title_weight = input.title_weight.unwrap_or(1);
    ensure!(
        (1..=16).contains(&title_weight),
        "titleWeight must be between 1 and 16"
    );
    for (i, &(start_byte, end_byte)) in chunks.iter().enumerate() {
        let text = &input.text[start_byte as usize..end_byte as usize];
        let source = keys.map_or(text, |k| k[i].as_str());
        ensure!(
            !options.positions || keys.is_none(),
            "positions require the original chunk text as the lexical key"
        );
        let (terms, word_count) = analyze_chunk(
            source,
            if i == 0 { title_weight } else { 1 },
            !options.unstemmed,
            options.positions.then_some(options.whole_document),
            analyzer,
        );
        let embedding = input
            .embeddings
            .as_ref()
            .map(|all| {
                normalize_embedding(&all[i], embedding_dimensions.unwrap_or(0))
                    .map(|values| Embedding::from_f64(&values))
            })
            .transpose()?;
        let (vector, vector_state) = match (mode, encoder.is_embedding(), &embedding) {
            (EncodeMode::Lexical, _, _) => (
                EntangledHVec::from_indices(Vec::new(), dimensions),
                VectorState::Lexical,
            ),
            (EncodeMode::Deferred, _, _) => (
                EntangledHVec::from_indices(Vec::new(), dimensions),
                VectorState::Pending,
            ),
            (EncodeMode::Eager, false, _) => {
                (encoder.encode(text, dimensions), VectorState::Encoded)
            }
            (EncodeMode::Eager, true, Some(e)) => (
                EntangledHVec::from_dense(e, dimensions),
                VectorState::Encoded,
            ),
            (EncodeMode::Eager, true, None) => bail!(
                "the {} text encoder needs a dense embedding for every chunk; install a text embedder",
                encoder.name()
            ),
        };
        stored.push(StoredChunk {
            vector,
            vector_state,
            text: input.store_text.unwrap_or(true).then(|| text.to_owned()),
            text_at: None,
            id: chunk_id(&input.id, i),
            start_byte,
            end_byte,
            embedding,
            terms,
            word_count,
            stemmed: !options.unstemmed,
            analyzed: !analyzer.is_legacy(),
        });
    }
    let mut document = StoredDocument {
        vector_record: false,
        id: input.id,
        source_uri: input.source_uri,
        version: input.version.unwrap_or_else(|| "1".into()),
        metadata: input.metadata.unwrap_or_else(|| serde_json::json!({})),
        chunks: stored,
    };
    compact_string(&mut document.id);
    compact_string(&mut document.version);
    if let Some(uri) = &mut document.source_uri {
        compact_string(uri);
    }
    compact_metadata(&mut document.metadata);
    compact_chunks(&mut document.chunks);
    Ok(document)
}

/// Identity of the content deferred encoding reads: the version, byte spans and text of every
/// chunk, mixed with the crate's splitmix64 chain. `None` for unretained text; storage errors propagate.
/// `Mutation::EncodeDocument` carries it so vectors computed for one revision of a document
/// are never installed on another that reused its id, version and chunk count.
pub(crate) fn document_fingerprint(
    store: &DocumentStore,
    document: &StoredDocument,
) -> Result<Option<u64>> {
    let mut h = super::super::encoding::hash_str_seed(&document.version, 0x5EED_D0C5_F1A6_0001);
    for chunk in &document.chunks {
        let Some(text) = store.try_chunk_text(chunk)? else {
            return Ok(None);
        };
        h = super::super::entangled::hash_u64(h, u64::from(chunk.start_byte));
        h = super::super::entangled::hash_u64(h, u64::from(chunk.end_byte));
        h = super::super::encoding::hash_str_seed(&text, h);
    }
    Ok(Some(h))
}
