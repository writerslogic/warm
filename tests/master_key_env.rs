// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
// Own test target: the test sets process environment variables.
#![cfg(feature = "security")]

//! `WARM_MASTER_KEY_PATH` (and the legacy `HMS_MASTER_KEY_PATH`) locate the master key; an
//! explicit config path wins over them.

use serde_json::json;
use writerslogic_warm::{core::WarmConfig, DocumentInput, SearchOptions, WarmCore};

fn open(path: &str, config: WarmConfig) -> anyhow::Result<WarmCore> {
    WarmCore::new(1024, Some(path.to_owned()), Some(config))
}

fn count(core: &WarmCore) -> usize {
    core.search_documents("canary", &SearchOptions::default())
        .unwrap()
        .len()
}

#[test]
fn environment_variable_locates_the_master_key() {
    let keys = tempfile::tempdir().unwrap();
    let explicit = keys.path().join("explicit.key");
    let current = keys.path().join("current/env.key");
    let legacy = keys.path().join("legacy.key");
    let cases: [(&str, &std::path::Path); 2] = [
        ("WARM_MASTER_KEY_PATH", &current),
        ("HMS_MASTER_KEY_PATH", &legacy),
    ];
    for (name, key_path) in cases {
        std::env::remove_var("WARM_MASTER_KEY_PATH");
        std::env::remove_var("HMS_MASTER_KEY_PATH");
        std::env::set_var(name, key_path);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        {
            let core = open(&path, WarmConfig::default()).unwrap();
            let doc: DocumentInput =
                serde_json::from_value(json!({ "id": "d", "text": "canary text" })).unwrap();
            core.memorize_document(doc).unwrap();
        }
        assert!(key_path.exists(), "{name}");
        assert!(!dir.path().join("master.key").exists(), "{name}");
        assert_eq!(count(&open(&path, WarmConfig::default()).unwrap()), 1);

        // The explicit option wins over the variable: the other path holds no key.
        let mut config = WarmConfig::default();
        config.security.master_key_file = Some(explicit.to_string_lossy().to_string());
        assert!(open(&path, config).is_err());
        assert!(!explicit.exists());

        // A missing key at the variable's path is an error, never a fresh key.
        std::fs::remove_file(key_path).unwrap();
        assert!(open(&path, WarmConfig::default()).is_err());
        assert!(!key_path.exists());
    }

    std::env::remove_var("HMS_MASTER_KEY_PATH");
    std::env::set_var("WARM_MASTER_KEY_PATH", "");
    let dir = tempfile::tempdir().unwrap();
    let error = open(&dir.path().to_string_lossy(), WarmConfig::default())
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("WARM_MASTER_KEY_PATH is empty"));
    std::env::remove_var("WARM_MASTER_KEY_PATH");
}
