// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn keyfile(root: &Path) -> PathBuf {
    root.join("keys").join("master.key")
}

fn encrypted(root: &Path) -> SealedStore {
    SealedStore::create(
        &root.join("store"),
        StoreOptions {
            mode: Some(Mode::Aes256Gcm),
            source: Some(KeySource::file(keyfile(root))),
        },
    )
    .unwrap()
}

fn reopen(root: &Path) -> anyhow::Result<SealedStore> {
    SealedStore::open(&root.join("store"), Some(KeySource::file(keyfile(root))))
}

fn fill(store: &SealedStore, blocks: u8) -> u64 {
    let mut w = store.create_segment(1024).unwrap();
    for i in 0..blocks {
        w.append_block(&vec![i + 1; 100 + usize::from(i)]).unwrap();
    }
    w.commit().unwrap();
    w.segment_id()
}

fn read_all(store: &SealedStore, id: u64) -> anyhow::Result<Vec<Vec<u8>>> {
    let r = store.open_segment_reader(id)?;
    (0..r.block_count())
        .map(|i| Ok(r.read_block(i)?.to_vec()))
        .collect()
}

fn flip(path: &Path, at: usize) {
    let mut bytes = std::fs::read(path).unwrap();
    bytes[at] ^= 0x01;
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn nonces_never_repeat_across_rewrite_and_reopen() {
    let root = tempfile::tempdir().unwrap();
    let store = encrypted(root.path());
    let mut w = store.create_segment(1024).unwrap();
    for i in 0..5u8 {
        w.append_block(&[i; 64]).unwrap();
    }
    w.rewrite_block(2, &[9; 64]).unwrap();
    w.rewrite_block(2, &[8; 64]).unwrap();
    w.commit().unwrap();
    let id = w.segment_id();
    let first: HashSet<u64> = w.counters().into_iter().collect();
    assert_eq!(first.len(), 5);
    let before = *first.iter().max().unwrap();
    // A block appended and never committed still consumed a counter.
    w.append_block(&[7; 64]).unwrap();
    drop(w);
    drop(store);

    let store = reopen(root.path()).unwrap();
    let mut w = store.open_segment_writer(id).unwrap();
    assert_eq!(w.block_count(), 5);
    w.append_block(&[6; 64]).unwrap();
    w.rewrite_block(0, &[5; 64]).unwrap();
    w.commit().unwrap();
    let all = w.counters();
    assert_eq!(all.iter().collect::<HashSet<_>>().len(), all.len());
    for c in all.iter().filter(|c| !first.contains(c)) {
        assert!(
            *c >= NONCE_CHUNK,
            "reopened writer resumed below the persisted mark (counter {c}, previous max {before})"
        );
    }
    drop(w);
    assert_eq!(read_all(&store, id).unwrap()[2], vec![8; 64]);
}

#[test]
fn any_tampered_byte_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let id = {
        let store = encrypted(root.path());
        fill(&store, 3)
    };
    let dir = root.path().join("store");
    let (dat, hdr) = segment::segment_paths(&dir, id, 1);
    let keys = dir.join("keys.tbl");

    for (path, what) in [(&dat, "data"), (&hdr, "header")] {
        for at in 0..std::fs::metadata(path).unwrap().len() as usize {
            flip(path, at);
            let store = reopen(root.path()).unwrap();
            assert!(read_all(&store, id).is_err(), "{what} byte {at} accepted");
            drop(store);
            flip(path, at);
        }
    }
    for at in 0..std::fs::metadata(&keys).unwrap().len() as usize {
        flip(&keys, at);
        assert!(reopen(root.path()).is_err(), "key table byte {at} accepted");
        flip(&keys, at);
    }
    assert_eq!(
        read_all(&reopen(root.path()).unwrap(), id).unwrap().len(),
        3
    );
}

#[test]
fn purge_recovers_from_a_crash_at_every_step() {
    // (failpoint, whether segment A survives the crash)
    let cases = [
        ("key.before_rename", true),
        ("rotate.staged", true),
        ("table.before_rename", true),
        ("table.after_rename", false),
        ("rotate.table_committed", false),
        ("rotate.after_scrub", false),
    ];
    for (point, survives) in cases {
        let root = tempfile::tempdir().unwrap();
        let (a, b, a_data) = {
            let store = encrypted(root.path());
            let (a, b) = (fill(&store, 2), fill(&store, 4));
            let a_data = read_all(&store, a).unwrap();
            arm_failpoint(Some(point));
            let result = store.purge_segment(a);
            arm_failpoint(None);
            assert!(result.is_err(), "{point} did not fire");
            (a, b, a_data)
        };
        let store = reopen(root.path()).unwrap_or_else(|e| panic!("{point}: reopen failed: {e}"));
        assert_eq!(read_all(&store, b).unwrap().len(), 4, "{point}");
        if survives {
            assert_eq!(read_all(&store, a).unwrap(), a_data, "{point}");
            store.purge_segment(a).unwrap();
        }
        assert!(store.open_segment_reader(a).is_err(), "{point}");
        drop(store);
        let store = reopen(root.path()).unwrap();
        assert_eq!(read_all(&store, b).unwrap().len(), 4, "{point}");
        let (dat, hdr) = segment::segment_paths(&root.path().join("store"), a, 1);
        assert!(
            !dat.exists() && !hdr.exists(),
            "{point}: purged files remain"
        );
        let mut next = keyfile(root.path()).into_os_string();
        next.push(".next");
        assert!(!PathBuf::from(next).exists(), "{point}: staged key remains");
    }
}

#[test]
fn old_key_table_is_useless_after_purge() {
    let root = tempfile::tempdir().unwrap();
    let store = encrypted(root.path());
    let (a, b) = (fill(&store, 2), fill(&store, 3));
    let keys = root.path().join("store").join("keys.tbl");
    let old_table = std::fs::read(&keys).unwrap();
    store.purge_segment(a).unwrap();
    let backup = root.path().join("backup.bin");
    store.export_backup(&backup, "backup passphrase").unwrap();
    drop(store);

    // The new master key against the pre-purge table: that table is wrapped under the old key.
    let current = std::fs::read(&keys).unwrap();
    std::fs::write(&keys, &old_table).unwrap();
    assert!(reopen(root.path()).is_err());
    std::fs::write(&keys, current).unwrap();

    let master: [u8; 32] = std::fs::read(keyfile(root.path()))
        .unwrap()
        .try_into()
        .unwrap();
    let restored_dir = root.path().join("restored");
    let restored = SealedStore::import_backup(
        &backup,
        "backup passphrase",
        &restored_dir,
        KeySource::raw(master),
    )
    .unwrap();
    for entry in std::fs::read_dir(root.path().join("store")).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name().to_string_lossy().starts_with("seg-") {
            std::fs::copy(entry.path(), restored_dir.join(entry.file_name())).unwrap();
        }
    }
    assert_eq!(read_all(&restored, b).unwrap().len(), 3);
    assert!(restored.open_segment_reader(a).is_err());
}

#[test]
fn plaintext_store_opens_and_migrates() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("store");
    let marker = b"PLAINTEXT-MARKER-0123456789";
    let id = {
        let store = SealedStore::create(
            &dir,
            StoreOptions {
                mode: Some(Mode::Plain),
                source: None,
            },
        )
        .unwrap();
        let mut w = store.create_segment(4096).unwrap();
        w.append_block(marker).unwrap();
        w.append_block(&[3; 500]).unwrap();
        w.commit().unwrap();
        w.segment_id()
    };
    let (dat1, _) = segment::segment_paths(&dir, id, 1);
    assert!(std::fs::read(&dat1)
        .unwrap()
        .windows(marker.len())
        .any(|w| w == marker));

    let store = SealedStore::open(&dir, None).unwrap();
    assert_eq!(store.mode(), Mode::Plain);
    assert_eq!(read_all(&store, id).unwrap()[0], marker);

    arm_failpoint(Some("migrate.before_commit"));
    let result = store.migrate_to_encrypted(Some(KeySource::file(keyfile(root.path()))));
    arm_failpoint(None);
    assert!(result.is_err());
    drop(store);
    let store = SealedStore::open(&dir, None).unwrap();
    assert_eq!(store.mode(), Mode::Plain);
    assert_eq!(read_all(&store, id).unwrap()[1], vec![3; 500]);
    assert!(!segment::segment_paths(&dir, id, 2).0.exists());

    store
        .migrate_to_encrypted(Some(KeySource::file(keyfile(root.path()))))
        .unwrap();
    drop(store);
    let store = reopen(root.path()).unwrap();
    assert_eq!(store.mode(), Mode::Aes256Gcm);
    assert_eq!(read_all(&store, id).unwrap()[0], marker);
    assert!(!dat1.exists());
    let (dat2, _) = segment::segment_paths(&dir, id, 2);
    assert!(!std::fs::read(dat2)
        .unwrap()
        .windows(marker.len())
        .any(|w| w == marker));
}
