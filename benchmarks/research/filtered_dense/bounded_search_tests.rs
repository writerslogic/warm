#[test]
fn bounded_filtered_snapshot_keeps_current_owners_across_mutation_and_store_drop() {
    let mut store = store(256);
    let trained = super::super::build_dense_index(
        super::super::capture_dense_index(&store, 3)
            .unwrap()
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert!(super::super::publish_dense_index(&mut store, &trained));
    let options = SearchOptions {
        embedding: Some(vec![1.0, 0.0, 0.0]),
        filter: Some(json!({"group": 0})),
        lexical_weight: Some(0.0),
        semantic_weight: Some(1.0),
        min_semantic_score: Some(-1.0),
        k: Some(10),
        candidate_limit: Some(100),
        ..Default::default()
    };
    let captured = capture_search(&store, vec![prepare_search("", &options, Some(3)).unwrap()]);
    assert_eq!(captured.queries[0].scan.as_ref().unwrap().len(), 128);
    let query = &captured.queries[0];
    assert!(query
        .dense
        .as_ref()
        .unwrap()
        .search_where_limited(
            query.prepared.embedding.as_ref().unwrap(),
            query.prepared.limit,
            query.prepared.minimum,
            |document| query.prepared.matches(document),
            query.count * 4 / 16,
        )
        .is_none());
    let expected = serde_json::to_value(
        super::super::search_impl(&store, "", &options, Some(3), false).unwrap(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(execute_search(&captured, 0).unwrap()).unwrap(),
        expected
    );
    let mut replacement = document(0, 99);
    replacement.metadata = json!({"group": 1});
    store.insert(replacement.id.clone(), replacement);
    store.remove("d002").unwrap();
    let added = document(1000, 1);
    store.insert(added.id.clone(), added);
    assert!(store.dense_index.is_some());
    for fusion in ["blend", "rrf"] {
        let options = SearchOptions {
            fusion: Some(fusion.into()),
            ..options.clone()
        };
        let fresh = capture_search(&store, vec![prepare_search("", &options, Some(3)).unwrap()]);
        let expected = serde_json::to_value(
            super::super::search_impl(&store, "", &options, Some(3), false).unwrap(),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(execute_search(&fresh, 0).unwrap()).unwrap(),
            expected
        );
    }
    drop(store);
    assert_eq!(
        serde_json::to_value(execute_search(&captured, 0).unwrap()).unwrap(),
        expected
    );
}
