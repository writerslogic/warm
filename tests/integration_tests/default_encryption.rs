// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//! A new store is sealed under a master key without any configuration; plaintext is an
//! explicit opt-out; a plaintext store from an earlier release keeps opening as plaintext. Reopening
//! with the wrong passphrase names the passphrase.

use serde_json::json;
use std::path::Path;
use writerslogic_warm::{core::WarmConfig, DocumentInput, SearchOptions, WarmCore};

const MARKER: &str = "zqxjdefaultsealedcanary";
const TEXT: &str = "alpha beta zqxjdefaultsealedcanary gamma delta";

fn plaintext() -> WarmConfig {
    let mut config = WarmConfig::default();
    config.security.plaintext = true;
    config
}

fn document() -> DocumentInput {
    serde_json::from_value(json!({ "id": "d1", "text": TEXT })).unwrap()
}

fn hit(core: &WarmCore) -> Option<String> {
    core.search_documents(MARKER, &SearchOptions::default())
        .unwrap()
        .into_iter()
        .find_map(|r| r.text)
}

fn files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn leaking(dir: &Path) -> Vec<String> {
    let mut all = Vec::new();
    files(dir, &mut all);
    all.into_iter()
        .filter(|p| {
            std::fs::read(p)
                .unwrap()
                .windows(MARKER.len())
                .any(|w| w == MARKER.as_bytes())
        })
        .map(|p| p.display().to_string())
        .collect()
}

fn open(path: &str, config: WarmConfig) -> anyhow::Result<WarmCore> {
    WarmCore::new(1024, Some(path.to_owned()), Some(config))
}

fn write_key(path: &Path, key: &[u8]) {
    let _ = std::fs::remove_file(path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(&mut options.open(path).unwrap(), key).unwrap();
}

fn schema(dir: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(dir.join("store.json")).unwrap()).unwrap()
}

#[test]
fn default_store_is_encrypted_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().to_string();
    {
        let core = open(&path, WarmConfig::default()).unwrap();
        core.memorize_document(document()).unwrap();
        assert_eq!(hit(&core).as_deref(), Some(TEXT));
        core.compact().unwrap();
        assert_eq!(hit(&core).as_deref(), Some(TEXT));
    }
    assert_eq!(schema(dir.path())["encrypted"], true);
    assert_eq!(schema(dir.path())["key_scheme"], "master-key");
    assert!(dir.path().join("master.key").exists());
    assert!(leaking(dir.path()).is_empty(), "{:?}", leaking(dir.path()));
    let core = open(&path, WarmConfig::default()).unwrap();
    assert_eq!(hit(&core).as_deref(), Some(TEXT));
    assert!(leaking(dir.path()).is_empty());
}

#[test]
fn plaintext_opt_out_writes_a_plaintext_store() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().to_string();
    {
        let core = open(&path, plaintext()).unwrap();
        core.memorize_document(document()).unwrap();
        core.compact().unwrap();
    }
    assert_eq!(schema(dir.path())["encrypted"], false);
    assert!(schema(dir.path()).get("key_scheme").is_none());
    assert!(!dir.path().join("master.key").exists());
    assert!(!leaking(dir.path()).is_empty());
}

#[test]
fn legacy_plaintext_store_opens_under_the_default_and_stays_plaintext() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().to_string();
    {
        let core = open(&path, plaintext()).unwrap();
        core.memorize_document(document()).unwrap();
    }
    let before = std::fs::read(dir.path().join("store.json")).unwrap();
    {
        // The default configuration adopts the protection the store records.
        let core = open(&path, WarmConfig::default()).unwrap();
        assert_eq!(hit(&core).as_deref(), Some(TEXT));
        core.memorize_document(
            serde_json::from_value(json!({ "id": "d2", "text": "second document" })).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(
        std::fs::read(dir.path().join("store.json")).unwrap(),
        before
    );
    assert!(!dir.path().join("master.key").exists());
    // Asking for encryption on an existing plaintext store is refused, not silently ignored.
    let mut config = WarmConfig::default();
    config.security.encryption_enabled = true;
    config.security.encryption_passphrase = Some("pw".into());
    assert!(open(&path, config).is_err());
    let core = open(&path, plaintext()).unwrap();
    assert_eq!(hit(&core).as_deref(), Some(TEXT));
}

#[test]
fn plaintext_opt_out_cannot_open_an_encrypted_store() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().to_string();
    drop(open(&path, WarmConfig::default()).unwrap());
    assert!(open(&path, plaintext()).is_err());
}

#[test]
fn wrong_or_missing_master_key_is_rejected_even_when_the_store_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().to_string();
    drop(open(&path, WarmConfig::default()).unwrap());
    let key_path = dir.path().join("master.key");
    let key = std::fs::read(&key_path).unwrap();

    write_key(&key_path, &[7u8; 32]);
    let error = format!("{:#}", open(&path, WarmConfig::default()).err().unwrap());
    assert!(error.contains("master key"), "{error}");

    // A missing key is never replaced by a fresh one.
    std::fs::remove_file(&key_path).unwrap();
    assert!(open(&path, WarmConfig::default()).is_err());
    assert!(!key_path.exists());

    write_key(&key_path, &key);
    drop(open(&path, WarmConfig::default()).unwrap());
}

#[test]
fn master_key_file_may_live_outside_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let keys = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().to_string();
    let mut config = WarmConfig::default();
    config.security.master_key_file = Some(keys.path().join("k.key").to_string_lossy().to_string());
    {
        let core = open(&path, config.clone()).unwrap();
        core.memorize_document(document()).unwrap();
    }
    assert!(!dir.path().join("master.key").exists());
    assert!(keys.path().join("k.key").exists());
    assert_eq!(hit(&open(&path, config).unwrap()).as_deref(), Some(TEXT));
}

#[test]
fn tampered_encrypted_store_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().to_string();
    {
        let core = open(&path, WarmConfig::default()).unwrap();
        core.memorize_document(document()).unwrap();
        core.compact().unwrap();
    }
    // Flip a ciphertext bit of the last frame and repair the frame's plaintext CRC32, so the
    // authentication tag is the only thing left to notice.
    let mut all = Vec::new();
    files(dir.path(), &mut all);
    let segment = all
        .iter()
        .find(|p| p.file_name().is_some_and(|n| n == "seg_0.bin"))
        .unwrap();
    let mut bytes = std::fs::read(segment).unwrap();
    let (mut at, mut last) = (0usize, None);
    while at + 16 <= bytes.len() {
        let word =
            |i: usize| u32::from_le_bytes(bytes[at + 4 * i..at + 4 * i + 4].try_into().unwrap());
        let (raw, comp) = (word(1) as usize, word(2) as usize);
        if comp == 0 || at + 16 + comp > bytes.len() {
            break;
        }
        assert_eq!(raw, comp, "ciphertext does not compress");
        last = Some((at, raw));
        at += 16 + comp;
    }
    let (start, len) = last.expect("the store holds frames");
    bytes[start + 16 + len - 1] ^= 0x01;
    let crc = crc32fast::hash(&bytes[start + 16..start + 16 + len]);
    bytes[start..start + 4].copy_from_slice(&crc.to_le_bytes());
    std::fs::write(segment, bytes).unwrap();
    let error = format!("{:#}", open(&path, WarmConfig::default()).err().unwrap());
    assert!(error.contains("Decryption failed"), "{error}");
}

#[test]
fn tampered_key_check_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().to_string();
    drop(open(&path, WarmConfig::default()).unwrap());
    let check = dir.path().join("key.check");
    let mut bytes = std::fs::read(&check).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    std::fs::write(&check, bytes).unwrap();
    assert!(open(&path, WarmConfig::default()).is_err());
}

fn passphrase_config(passphrase: &str) -> WarmConfig {
    let mut config = WarmConfig::default();
    config.security.encryption_enabled = true;
    config.security.encryption_passphrase = Some(passphrase.to_owned());
    config
}

#[test]
fn wrong_passphrase_error_mentions_the_passphrase() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().to_string();
    {
        let core = WarmCore::new(
            1024,
            Some(path.clone()),
            Some(passphrase_config("right one")),
        )
        .unwrap();
        let doc: DocumentInput =
            serde_json::from_value(json!({ "id": "d1", "text": "alpha beta" })).unwrap();
        core.memorize_document(doc).unwrap();
    }

    let error = match WarmCore::new(
        1024,
        Some(path.clone()),
        Some(passphrase_config("wrong passphrase")),
    ) {
        Ok(_) => panic!("opening with the wrong passphrase must fail"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("passphrase"), "{error}");
    assert!(!error.contains("right one"), "{error}");

    // the right passphrase still opens it
    WarmCore::new(1024, Some(path), Some(passphrase_config("right one"))).unwrap();
}
