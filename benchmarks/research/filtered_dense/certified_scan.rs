fn certified_dot_bounds(query: &[f64], values: &[f32]) -> (f64, f64) {
    if values.len() != query.len() || !(1..=4096).contains(&values.len()) {
        return (-1.0, 1.0);
    }
    let mut sums = [-0.0_f64; 4];
    let mut maxima = [0.0_f32; 4];
    let mut vectors = values.chunks_exact(4);
    let mut queries = query.chunks_exact(4);
    for (row, q) in vectors.by_ref().zip(queries.by_ref()) {
        for lane in 0..4 {
            sums[lane] += f64::from(row[lane]) * q[lane];
            maxima[lane] = maxima[lane].max(row[lane].abs());
        }
    }
    for (&value, &q) in vectors.remainder().iter().zip(queries.remainder()) {
        sums[0] += f64::from(value) * q;
        maxima[0] = maxima[0].max(value.abs());
    }
    let estimate = sums.into_iter().sum::<f64>();
    let maximum = maxima.into_iter().fold(0.0_f32, f32::max);
    if !estimate.is_finite() || !maximum.is_finite() {
        return (-1.0, 1.0);
    }
    let dimensions = values.len() as f64;
    let radius = 8.0 * dimensions * dimensions * f64::EPSILON * f64::from(maximum);
    (
        (estimate - radius).next_down().clamp(-1.0, 1.0),
        (estimate + radius).next_up().clamp(-1.0, 1.0),
    )
}

#[cfg(test)]
thread_local! {
    static CERTIFIED_SCAN_COUNTS: std::cell::Cell<Option<(usize, usize)>> = const { std::cell::Cell::new(None) };
}

fn execute_semantic_scan(
    captured: &CapturedQuery,
    documents: &[Arc<StoredDocument>],
) -> Vec<RankedHit> {
    let prepared = &captured.prepared;
    assert!(
        prepared.pure_semantic(),
        "semantic scan captured a lexical query"
    );
    let query = prepared
        .embedding
        .as_ref()
        .expect("semantic scan lost its embedding");
    let mut candidates = Vec::with_capacity(captured.count);
    for (owner, document) in documents.iter().enumerate() {
        for (index, chunk) in document.chunks.iter().enumerate() {
            if let Some(embedding) = &chunk.embedding {
                let (lower, upper) = certified_dot_bounds(query, embedding);
                candidates.push((lower, upper, owner, index));
            }
        }
    }
    let threshold = if candidates.len() > prepared.limit {
        let (_, boundary, _) =
            candidates.select_nth_unstable_by(prepared.limit - 1, |a, b| b.0.total_cmp(&a.0));
        boundary.0.max(prepared.minimum)
    } else {
        prepared.minimum
    };
    #[cfg(test)]
    let screened = candidates.len();
    candidates.retain(|candidate| candidate.1 >= threshold);
    #[cfg(test)]
    CERTIFIED_SCAN_COUNTS.set(Some((screened, candidates.len())));
    candidates.retain_mut(|candidate| {
        let embedding = documents[candidate.2].chunks[candidate.3]
            .embedding
            .as_ref()
            .expect("screened row lost its embedding");
        candidate.0 = embedding.cosine(query);
        candidate.0 >= prepared.minimum
    });
    top_ranked(&mut candidates, prepared.limit, |a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| (&documents[a.2].id, a.3).cmp(&(&documents[b.2].id, b.3)))
    });
    let chunks: Vec<_> = candidates
        .into_iter()
        .map(|(score, _, owner, index)| {
            let document = &documents[owner];
            CapturedChunk {
                document: Arc::clone(document),
                index,
                word_count: super::index::slot_words(document.chunks[index].word_count),
                semantic_cache: Some(score),
            }
        })
        .collect();
    execute_chunks(captured, &chunks)
}
