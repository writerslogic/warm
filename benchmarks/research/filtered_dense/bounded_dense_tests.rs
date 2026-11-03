#[test]
fn bounded_root_and_full_traversal_work_boundaries_never_return_partial_hits() {
    let mut negative = DocumentStore::default();
    for id in 0..128 {
        let document = document(id, &[vec![-1.0, 0.0]]);
        negative.insert(document.id.clone(), document);
    }
    let trained = train(&negative, 2);
    assert!(trained
        .index
        .search_where_limited(&[1.0, 0.0], 10, 0.0, |_| true, 1)
        .is_none());
    for work in [2, 3] {
        let (hits, dots) = trained
            .index
            .search_where_limited(&[1.0, 0.0], 10, 0.0, |_| true, work)
            .unwrap();
        assert!(hits.is_empty());
        assert_eq!(dots, 0);
    }
    let mut equal = DocumentStore::default();
    for id in 0..129 {
        let document = document(id, &[vec![1.0, 0.0, 0.0, 0.0]]);
        equal.insert(document.id.clone(), document);
    }
    let trained = train(&equal, 4);
    let index = &trained.index;
    let query = [1.0, 0.0, 0.0, 0.0];
    let required = index.nodes.len() * 4 + index.rows.len() * 5;
    for work in [0, 1, 4, required - 1] {
        assert!(index
            .search_where_limited(&query, 10, -1.0, |_| true, work)
            .is_none());
    }
    let expected = exhaustive(index, &query, 10, -1.0);
    for work in [required, required + 1] {
        let (hits, dots) = index
            .search_where_limited(&query, 10, -1.0, |_| true, work)
            .unwrap();
        assert_eq!(actual(hits), expected);
        assert_eq!(dots, 129);
    }
}

#[test]
fn bounded_work_retains_useful_separated_filtered_pruning() {
    let mut store = DocumentStore::default();
    for id in 0..8192 {
        let vector = if id < 64 {
            vec![1.0, 0.0, 0.0, 0.0]
        } else {
            vec![-1.0, (id % 5) as f64 / 100.0, 0.0, 0.0]
        };
        let document = document(id, &[vector]);
        store.insert(document.id.clone(), document);
    }
    let trained = train(&store, 4);
    let query = [1.0, 0.0, 0.0, 0.0];
    let matches = |document: &StoredDocument| document.metadata["group"] == 0;
    let (expected, original_dots) = trained.index.search_where(&query, 10, -1.0, matches);
    let (hits, dots) = trained
        .index
        .search_where_limited(&query, 10, -1.0, matches, 4096 * 5 / 16)
        .unwrap();
    assert_eq!(actual(hits), actual(expected));
    assert_eq!(dots, original_dots);
    assert!(dots < 4096);
}
