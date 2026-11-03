#[test]
fn computed_bounds_are_tight_for_point_boxes_at_dimension_boundaries() {
    for dimensions in [1, 2, 3, 31, 32, 33, 95, 96, 97, 511, 512, 513, 4095, 4096] {
        let vectors = [
            (0..dimensions)
                .map(|i| [1.0, f32::from_bits(1), -1.0, -0.0][i % 4])
                .collect::<Vec<_>>(),
            (0..dimensions)
                .map(|i| [f32::MAX, -f32::MAX, f32::MIN_POSITIVE, -f32::MIN_POSITIVE][i % 4])
                .collect(),
        ];
        for vector in vectors {
            let mut store = DocumentStore::default();
            for id in 0..129 {
                let mut item = document(id, &[vec![1.0; dimensions]]);
                item.chunks[0].embedding = Some(documents::Embedding::from_f32(vector.clone()));
                store.insert(item.id.clone(), item);
            }
            let trained = train(&store, dimensions);
            let index = &trained.index;
            for raw in [
                vec![1.0; dimensions],
                (0..dimensions)
                    .map(|i| [1.0, f64::from_bits(1), -1.0, -0.0][i % 4])
                    .collect(),
            ] {
                let query = documents::normalize_embedding(&raw, dimensions).unwrap();
                let score = index.rows[0].stored().cosine(&query);
                assert_eq!(index.upper(0, &query), score, "dimension={dimensions}");
                assert!(index.upper_baseline(0, &query) >= score);
                for minimum in [score.next_down().max(-1.0), score, score.next_up().min(1.0)] {
                    for limit in [9, 10, 11] {
                        assert_eq!(
                            actual(index.search(&query, limit, minimum).0),
                            exhaustive(index, &query, limit, minimum)
                        );
                    }
                }
            }
        }
    }
}
