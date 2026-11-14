// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! Opt-in learned-sparse ranking through the document API.

use anyhow::Result;
use serde_json::json;
use std::path::Path;
use writerslogic_warm::core::WarmConfig;
use writerslogic_warm::{
    DocumentInput, RankingMode, SearchOptions, SparseSearch, SparseVector, WarmCore,
};

const FILE: &str = "sparse-docs.wsp";

fn open(dir: &Path) -> WarmCore {
    WarmCore::new(
        1024,
        Some(dir.to_string_lossy().into_owned()),
        Some(WarmConfig::default()),
    )
    .unwrap()
}

fn doc(id: &str, text: &str) -> DocumentInput {
    serde_json::from_value(json!({ "id": id, "text": text })).unwrap()
}

fn vector(pairs: &[(u32, f32)]) -> SparseVector {
    SparseVector::new(
        pairs.iter().map(|p| p.0).collect(),
        pairs.iter().map(|p| p.1).collect(),
    )
}

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }

    fn vector(&mut self, dims: u64, max_nnz: u64) -> SparseVector {
        let mut ids: Vec<u32> = (0..1 + self.below(max_nnz))
            .map(|_| self.below(dims) as u32)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let weights = ids
            .iter()
            .map(|_| 0.05 + self.below(500) as f32 / 50.0)
            .collect();
        SparseVector::new(ids, weights)
    }
}

fn sparse(query: SparseVector) -> SparseSearch {
    SparseSearch {
        mode: RankingMode::Sparse,
        query,
    }
}

fn sparse_ids(core: &WarmCore, query: SparseVector, k: u32) -> Vec<String> {
    let options = SearchOptions {
        k: Some(k),
        ..Default::default()
    };
    core.search_documents_ranked("", &options, &sparse(query))
        .unwrap()
        .into_iter()
        .map(|r| r.document_id)
        .collect()
}

#[test]
fn default_store_writes_no_sparse_file_and_lexical_mode_is_unchanged() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let core = open(dir.path());
    core.memorize_documents(vec![
        doc("a", "alpha beta gamma"),
        doc("b", "beta delta"),
        doc("c", "gamma gamma epsilon"),
    ])?;
    let options = SearchOptions {
        k: Some(5),
        ..Default::default()
    };
    let plain = core.search_documents("gamma beta", &options)?;
    let ranked = core.search_documents_ranked("gamma beta", &options, &SparseSearch::default())?;
    assert_eq!(
        serde_json::to_string(&plain)?,
        serde_json::to_string(&ranked)?
    );
    drop(core);
    let core = open(dir.path());
    assert_eq!(core.sparse_document_count(), 0);
    assert!(!dir.path().join(FILE).exists());
    Ok(())
}

#[test]
fn sparse_ranking_equals_brute_force_and_survives_reopen() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let vectors: Vec<SparseVector> = (0..150).map(|_| rng.vector(400, 40)).collect();
    let queries: Vec<SparseVector> = (0..20).map(|_| rng.vector(400, 12)).collect();
    let expected = |q: &SparseVector, k: usize| -> Vec<String> {
        let mut scored: Vec<(String, f64)> = vectors
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let s: f64 = v
                    .ids
                    .iter()
                    .zip(&v.weights)
                    .filter_map(|(t, w)| {
                        let at = q.ids.iter().position(|x| x == t)?;
                        Some(f64::from(*w) * f64::from(q.weights[at]))
                    })
                    .sum();
                (format!("d{i:03}"), s)
            })
            .filter(|p| p.1 > 0.0)
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        scored.into_iter().take(k).map(|p| p.0).collect()
    };
    let check = |core: &WarmCore| {
        for q in &queries {
            for k in [1, 10, 200] {
                assert_eq!(sparse_ids(core, q.clone(), k as u32), expected(q, k));
            }
        }
    };
    {
        let core = open(dir.path());
        let inputs = vectors
            .iter()
            .enumerate()
            .map(|(i, v)| {
                (
                    doc(&format!("d{i:03}"), &format!("word{i} shared")),
                    Some(v.clone()),
                )
            })
            .collect();
        core.memorize_documents_sparse(inputs, &Default::default())?;
        check(&core);
    }
    check(&open(dir.path()));
    Ok(())
}

#[test]
fn deleted_and_replaced_documents_never_surface() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let q = vector(&[(1, 1.0)]);
    let core = open(dir.path());
    let opts = Default::default();
    core.memorize_documents_sparse(
        vec![
            (doc("a", "one"), Some(vector(&[(1, 3.0)]))),
            (doc("b", "two"), Some(vector(&[(1, 2.0)]))),
            (doc("c", "three"), Some(vector(&[(1, 1.0)]))),
        ],
        &opts,
    )?;
    assert_eq!(sparse_ids(&core, q.clone(), 10), ["a", "b", "c"]);
    core.delete_document("a")?;
    assert_eq!(sparse_ids(&core, q.clone(), 10), ["b", "c"]);
    core.memorize_document_sparse(doc("b", "two again"), None, &opts)?;
    assert_eq!(sparse_ids(&core, q.clone(), 10), ["c"]);
    core.memorize_document_sparse(doc("b", "two third"), Some(vector(&[(1, 9.0)])), &opts)?;
    assert_eq!(sparse_ids(&core, q.clone(), 10), ["b", "c"]);
    core.memorize_documents(vec![doc("c", "plain replacement")])?;
    assert_eq!(sparse_ids(&core, q.clone(), 10), ["b"]);
    drop(core);
    assert_eq!(sparse_ids(&open(dir.path()), q, 10), ["b"]);
    Ok(())
}

#[test]
fn crash_between_document_and_sparse_write_reopens_cleanly() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let q = vector(&[(7, 1.0)]);
    {
        let core = open(dir.path());
        core.memorize_documents_sparse(
            vec![
                (doc("kept", "x"), Some(vector(&[(7, 2.0)]))),
                (doc("torn", "y"), Some(vector(&[(7, 5.0), (8, 1.0)]))),
            ],
            &Default::default(),
        )?;
        core.memorize_document_sparse(doc("late", "z"), None, &Default::default())?;
    }
    let path = dir.path().join(FILE);
    let whole = std::fs::read(&path)?;
    for cut in [1, 5, 20] {
        std::fs::write(&path, &whole[..whole.len() - cut])?;
        let core = open(dir.path());
        assert!(core.has_document("torn"));
        assert_eq!(sparse_ids(&core, q.clone(), 10), ["kept"], "cut {cut}");
        drop(core);
    }
    std::fs::write(&path, &whole)?;
    let ghost = {
        let store = writerslogic_warm::core::sparse_store::SparseStore::open(dir.path())?;
        store.put("ghost", &vector(&[(7, 100.0)]))?;
        store.len()
    };
    assert_eq!(ghost, 3);
    assert_eq!(sparse_ids(&open(dir.path()), q, 10), ["torn", "kept"]);
    Ok(())
}

#[test]
fn fused_ranking_combines_text_and_sparse_lists() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let core = open(dir.path());
    core.memorize_documents_sparse(
        vec![
            (doc("both", "quartz topaz"), Some(vector(&[(3, 4.0)]))),
            (doc("text-only", "quartz quartz quartz"), None),
            (
                doc("vec-only", "unrelated words"),
                Some(vector(&[(3, 3.0)])),
            ),
            (doc("neither", "nothing here"), Some(vector(&[(9, 1.0)]))),
        ],
        &Default::default(),
    )?;
    let ranking = SparseSearch {
        mode: RankingMode::Fused,
        query: vector(&[(3, 1.0)]),
    };
    let options = SearchOptions {
        k: Some(3),
        ..Default::default()
    };
    let hits = core.search_documents_ranked("quartz topaz", &options, &ranking)?;
    let ids: Vec<&str> = hits.iter().map(|h| h.document_id.as_str()).collect();
    assert_eq!(ids[0], "both");
    assert!(ids.contains(&"text-only") && ids.contains(&"vec-only"));
    assert!(hits.windows(2).all(|w| w[0].score >= w[1].score));
    let filtered = SearchOptions {
        source_uri: Some("x".into()),
        ..options
    };
    assert!(core
        .search_documents_ranked("quartz", &filtered, &ranking)
        .is_err());
    Ok(())
}

#[cfg(feature = "security")]
#[test]
fn encrypted_store_refuses_sparse_vectors_and_writes_no_file() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut config = WarmConfig::default();
    config.security.encryption_enabled = true;
    config.security.encryption_passphrase = Some("pw".into());
    let core = WarmCore::new(
        1024,
        Some(dir.path().to_string_lossy().into_owned()),
        Some(config),
    )?;
    assert!(core
        .memorize_document_sparse(
            doc("a", "t"),
            Some(vector(&[(1, 1.0)])),
            &Default::default()
        )
        .is_err());
    assert!(!core.has_document("a"));
    core.memorize_document_sparse(doc("b", "t"), None, &Default::default())?;
    assert!(!dir.path().join(FILE).exists());
    Ok(())
}
