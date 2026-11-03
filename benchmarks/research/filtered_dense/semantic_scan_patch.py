"""Create the isolated candidate source and preserved control functions."""


def replace_once(text, old, new):
    if text.count(old) != 1:
        raise ValueError('source anchor changed: ' + old[:80])
    return text.replace(old, new, 1)


def relocate_control(text):
    for module in ('dense', 'index'):
        text = text.replace('super::' + module + '::', 'crate::core::documents::' + module + '::')
    return text


def patch(original):
    baseline_capture = original[original.index('fn capture_query('):original.index('/// Captures the chunks')]
    baseline_capture = baseline_capture.replace('fn capture_query(', 'fn baseline_capture_query(').replace('capture_planned_query(', 'baseline_capture_planned_query(')
    baseline_plan = original[original.index('fn capture_planned_query('):original.index('impl CapturedSearch {')]
    baseline_plan = baseline_plan.replace('fn capture_planned_query(', 'fn baseline_capture_planned_query(')
    baseline_execute = original[original.index('fn execute_query('):original.index('fn execute_chunks(')]
    baseline_execute = baseline_execute.replace('fn execute_query(', 'fn baseline_execute_query(')
    candidate = replace_once(original, 'impl PreparedSearch {\n', '''impl PreparedSearch {
    fn pure_semantic(&self) -> bool {
        self.embedding.is_some()
            && self.lexical_weight == 0.0
            && self.query_terms.iter().all(BTreeSet::is_empty)
    }

''')
    candidate = replace_once(candidate, '''    if prepared.lexical_only {
        return capture_ranked_lexical(store, prepared);
    }''', '''    if !prepared.unfiltered && prepared.pure_semantic() {
        return capture_planned_query(store, prepared, None);
    }
    if prepared.lexical_only {
        return capture_ranked_lexical(store, prepared);
    }''')
    candidate = replace_once(candidate, 'if count < super::dense::MIN_ROWS {',
                             'if count < super::dense::MIN_ROWS && !prepared.pure_semantic() {')
    candidate = replace_once(candidate, '''        if dense.is_some()
            && prepared.lexical_weight == 0.0
            && prepared.query_terms.iter().all(BTreeSet::is_empty)
        {''', '''        if prepared.pure_semantic() {''')
    candidate = replace_once(candidate, '''                None => {
                    let mut chunks = Vec::with_capacity(captured.count);
                    for document in documents {
                        for (index, chunk) in document.chunks.iter().enumerate() {
                            chunks.push(CapturedChunk {
                                document: Arc::clone(document),
                                index,
                                word_count: super::index::slot_words(chunk.word_count),
                                semantic_cache: None,
                            });
                        }
                    }
                    return execute_chunks(captured, &chunks);
                }''', '''                None => return execute_semantic_scan(captured, documents),''')
    candidate = replace_once(candidate, '''    execute_chunks(captured, &captured.chunks)
}

fn execute_chunks''', '''    if let Some(documents) = &captured.scan {
        return execute_semantic_scan(captured, documents);
    }
    execute_chunks(captured, &captured.chunks)
}

fn execute_semantic_scan(
    captured: &CapturedQuery,
    documents: &[Arc<StoredDocument>],
) -> Vec<RankedHit> {
    let prepared = &captured.prepared;
    assert!(prepared.pure_semantic(), "semantic scan captured a lexical query");
    let query = prepared.embedding.as_ref().expect("semantic scan lost its embedding");
    let mut candidates = Vec::with_capacity(captured.count);
    for (owner, document) in documents.iter().enumerate() {
        for (index, chunk) in document.chunks.iter().enumerate() {
            if let Some(embedding) = &chunk.embedding {
                let score = embedding.cosine(query);
                if score >= prepared.minimum {
                    candidates.push((score, owner, index));
                }
            }
        }
    }
    top_ranked(&mut candidates, prepared.limit, |a, b| {
        b.0.total_cmp(&a.0).then_with(|| {
            (&documents[a.1].id, a.2).cmp(&(&documents[b.1].id, b.2))
        })
    });
    let chunks: Vec<_> = candidates.into_iter().map(|(score, owner, index)| {
        let document = &documents[owner];
        CapturedChunk {
            document: Arc::clone(document),
            index,
            word_count: super::index::slot_words(document.chunks[index].word_count),
            semantic_cache: Some(score),
        }
    }).collect();
    execute_chunks(captured, &chunks)
}

fn execute_chunks''')
    baseline = baseline_capture + baseline_plan + baseline_execute
    return candidate, relocate_control(baseline)
