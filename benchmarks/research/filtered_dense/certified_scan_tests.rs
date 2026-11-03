#[test]
fn certified_intervals_contain_canonical_scores_at_arithmetic_boundaries() {
    let mut seed = 0xd1b5_4a32_d192_ed03_u64;
    for dimensions in [
        1, 2, 3, 4, 5, 31, 32, 33, 95, 96, 97, 511, 512, 513, 4095, 4096,
    ] {
        for pattern in 0..21 {
            let values: Vec<_> = (0..dimensions)
                .map(|i| {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    match pattern {
                        0 => [1.0, f32::from_bits(1), -1.0, -0.0][i % 4],
                        1 => [f32::MAX, -f32::MAX, f32::MIN_POSITIVE, -f32::MIN_POSITIVE][i % 4],
                        2 => [0.0, -0.0, 0.0, -0.0][i % 4],
                        3 => [
                            f32::from_bits(1),
                            -f32::from_bits(1),
                            f32::MIN_POSITIVE,
                            0.0,
                        ][i % 4],
                        4 => [0.25, 0.25_f32.next_up(), -0.25, 0.25_f32.next_down()][i % 4],
                        _ => f32::from_bits(
                            (seed as u32 & 0x807f_ffff) | (((seed >> 32) % 255) as u32) << 23,
                        ),
                    }
                })
                .collect();
            let embedding = super::super::Embedding::from_f32(values.clone());
            for form in 0..3 {
                let raw: Vec<_> = (0..dimensions)
                    .map(|i| match form {
                        0 => {
                            if i % 2 == 0 {
                                1.0
                            } else {
                                -1.0
                            }
                        }
                        1 => {
                            if i == 0 {
                                1.0
                            } else {
                                f64::from_bits(1)
                            }
                        }
                        _ => [1.0, 1e-8, -1.0, -0.0][i % 4],
                    })
                    .collect();
                let query = super::super::normalize_embedding(&raw, dimensions).unwrap();
                let score = embedding.cosine(&query);
                let (lower, upper) = certified_dot_bounds(&query, &values);
                assert!(score.is_finite());
                assert!(
                    lower <= score && score <= upper,
                    "d={dimensions} pattern={pattern} form={form}: {lower} <= {score} <= {upper}"
                );
                if pattern != 1 && values.iter().all(|value| value.abs() <= 1.0) {
                    assert!(upper - lower < 1e-6, "uninformative interval");
                }
            }
        }
    }
    let values = vec![1.0, 1e-20, -1.0, 0.0, 1e-20, 0.0, 0.0, 0.0];
    let query = super::super::normalize_embedding(&[1.0; 8], 8).unwrap();
    let score = super::super::Embedding::from_f32(values.clone()).cosine(&query);
    let (lower, upper) = certified_dot_bounds(&query, &values);
    assert!(score > 0.0);
    assert_ne!(
        score,
        (lower + upper) * 0.5,
        "fixture must exercise a changed reduction result"
    );
    for dimensions in [0, 4097] {
        assert_eq!(
            certified_dot_bounds(&vec![1.0; dimensions], &vec![1.0; dimensions]),
            (-1.0, 1.0)
        );
    }
    assert_eq!(certified_dot_bounds(&[1.0], &[1.0, 0.0]), (-1.0, 1.0));
    assert_eq!(certified_dot_bounds(&[1.0], &[f32::INFINITY]), (-1.0, 1.0));
    assert_eq!(certified_dot_bounds(&[1.0], &[f32::NAN]), (-1.0, 1.0));
}

#[test]
fn certified_selection_preserves_near_ties_and_filter_intersections() {
    for dimensions in [2, 4, 96, 512] {
        let mut store = DocumentStore::default();
        for id in 0..201 {
            let mut seed_vector = vec![0.0; dimensions];
            seed_vector[0] = 1.0;
            let input = serde_json::from_value(json!({
                "id": format!("d{id:03}"), "text": "certified near ties", "sourceUri": "certified-fixture",
                "metadata": {"group": 0}, "embeddings": [seed_vector]
            })).unwrap();
            let mut document =
                super::super::prepare(input, 1024, Some(dimensions), TextEncoder::default())
                    .unwrap();
            let mut values = vec![0.0; dimensions];
            values[0] = 0.25;
            values[1] = f32::from_bits(0.25_f32.to_bits() + id);
            document.chunks[0].embedding = Some(super::super::Embedding::from_f32(values));
            store.insert(document.id.clone(), document);
        }
        for trained in [false, true] {
            if trained {
                bounded_train(&mut store, dimensions);
            }
            for second in [0.0, 1e-8] {
                let mut query = vec![0.0; dimensions];
                query[0] = 1.0;
                query[1] = second;
                for limit in [99, 100, 101] {
                    for with_ids in [false, true] {
                        for fusion in ["blend", "rrf"] {
                            for minimum in [-1.0, 0.25, 0.250000003] {
                                let options = SearchOptions {
                                    k: Some(10),
                                    candidate_limit: Some(limit),
                                    embedding: Some(query.clone()),
                                    lexical_weight: Some(0.0),
                                    semantic_weight: Some(1.0),
                                    min_semantic_score: Some(minimum),
                                    filter: Some(json!({"group": 0})),
                                    source_uri: Some("certified-fixture".into()),
                                    document_ids: with_ids.then(|| {
                                        (0..201).step_by(2).map(|id| format!("d{id:03}")).collect()
                                    }),
                                    fusion: Some(fusion.into()),
                                    ..Default::default()
                                };
                                let captured = capture_search(
                                    &store,
                                    vec![prepare_search("", &options, Some(dimensions)).unwrap()],
                                );
                                let expected = super::super::search_impl(
                                    &store,
                                    "",
                                    &options,
                                    Some(dimensions),
                                    false,
                                )
                                .unwrap();
                                CERTIFIED_SCAN_COUNTS.set(None);
                                let actual = execute_search(&captured, 0).unwrap();
                                assert_eq!(serde_json::to_value(actual).unwrap(), serde_json::to_value(expected).unwrap(),
                                    "d={dimensions} trained={trained} limit={limit} ids={with_ids} fusion={fusion} minimum={minimum}");
                                if !trained && second == 0.0 && minimum <= 0.25 {
                                    let (screened, rescored) = CERTIFIED_SCAN_COUNTS
                                        .get()
                                        .expect("untrained filtered query must scan");
                                    assert_eq!(screened, if with_ids { 101 } else { 201 });
                                    assert_eq!(screened, rescored, "equal scores must not be discarded by the arithmetic shortcut");
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
