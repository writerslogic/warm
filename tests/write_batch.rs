// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Atomic mixed batches (`WarmCore::write_batch`): all of a batch or none of it, after a
//! failed validation, a crash at any byte of the journal append, and for a concurrent reader.

use std::path::{Path, PathBuf};
use writerslogic_warm::core::documents::{BatchTooLarge, DocumentInput};
use writerslogic_warm::core::WarmConfig;
use writerslogic_warm::{BatchOp, BatchResult, ReadOnlyError, WarmCore};

const DIM: u32 = 256;

fn plain() -> WarmConfig {
    WarmConfig::unencrypted()
}

#[cfg(feature = "security")]
fn sealed() -> WarmConfig {
    let mut config = WarmConfig::unencrypted();
    config.security.plaintext = false;
    config.security.encryption_enabled = true;
    config.security.encryption_passphrase = Some("write-batch-test-passphrase".into());
    config
}

#[allow(unused_mut)]
fn configs() -> Vec<(&'static str, WarmConfig)> {
    let mut all = vec![("plain", plain())];
    #[cfg(feature = "security")]
    all.push(("sealed", sealed()));
    all
}

fn open(dir: &Path, config: &WarmConfig) -> anyhow::Result<WarmCore> {
    WarmCore::new(DIM, Some(dir.display().to_string()), Some(config.clone()))
}

fn open_reader(dir: &Path, config: &WarmConfig) -> anyhow::Result<WarmCore> {
    WarmCore::open_read_only(DIM, Some(dir.display().to_string()), Some(config.clone()))
}

fn input(id: &str, text: &str) -> DocumentInput {
    serde_json::from_value(serde_json::json!({ "id": id, "text": text })).unwrap()
}

fn put(id: &str, text: &str) -> BatchOp {
    BatchOp::Memorize {
        document: input(id, text),
    }
}

fn del(id: &str) -> BatchOp {
    BatchOp::Delete { id: id.into() }
}

const UNIVERSE: [&str; 7] = ["keep", "edit", "gone", "swap", "new1", "new2", "late"];

/// Everything a reader of the store can observe about the batch's documents.
fn snapshot(core: &WarmCore) -> String {
    let mut out = format!("count={}\n", core.document_count());
    for id in UNIVERSE {
        let record = core.get_document(id).unwrap();
        out += &format!("{id}={}\n", serde_json::to_string(&record).unwrap());
    }
    let hits = core
        .search_documents("zebra", &Default::default())
        .unwrap()
        .into_iter()
        .map(|hit| format!("{}:{}", hit.document_id, hit.version))
        .collect::<Vec<_>>();
    out + &format!("zebra={hits:?}")
}

fn seed(core: &WarmCore) {
    core.memorize_documents(vec![
        input("keep", "keep zebra stays"),
        input("edit", "edit zebra original"),
        input("gone", "gone zebra removed"),
        input("swap", "swap zebra before"),
    ])
    .unwrap();
}

fn batch() -> Vec<BatchOp> {
    vec![
        put("new1", "new1 zebra added"),
        del("gone"),
        put("edit", "edit zebra replaced entirely"),
        del("swap"),
        put("swap", "swap zebra after"),
        put("new2", "new2 zebra added too"),
        del("absent"),
    ]
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == ".hms.lock" {
            continue;
        }
        let target = to.join(&name);
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn segment(store: &Path) -> PathBuf {
    let arena = store.join("vectors_data.bin");
    let live = match std::fs::read_to_string(arena.join("CURRENT")) {
        Ok(name) => arena.join(name),
        Err(_) => arena,
    };
    live.join("seg_0.bin")
}

#[test]
fn results_report_each_operation_in_order() {
    for (name, config) in configs() {
        let dir = tempfile::tempdir().unwrap();
        let core = open(dir.path(), &config).unwrap();
        seed(&core);
        let results = core.write_batch(batch()).unwrap();
        assert_eq!(
            results,
            [
                BatchResult::Memorize { chunks: 1 },
                BatchResult::Delete { existed: true },
                BatchResult::Memorize { chunks: 1 },
                BatchResult::Delete { existed: true },
                BatchResult::Memorize { chunks: 1 },
                BatchResult::Memorize { chunks: 1 },
                BatchResult::Delete { existed: false },
            ],
            "{name}"
        );
        assert!(core.get_document("gone").unwrap().is_none(), "{name}");
        assert!(core.get_document("new1").unwrap().is_some(), "{name}");
        assert!(
            core.get_document("edit")
                .unwrap()
                .is_some_and(|d| d.text.as_deref().is_some_and(|t| t.contains("replaced"))),
            "{name}"
        );
        drop(core);
        let core = open(dir.path(), &config).unwrap();
        assert!(core.get_document("gone").unwrap().is_none(), "{name}");
        assert!(core.get_document("swap").unwrap().is_some(), "{name}");
        assert_eq!(core.document_count(), 5, "{name}");
    }
}

/// Operations apply in order: delete then memorize of an id leaves the new document, memorize
/// then delete leaves none, a repeated delete reports the id absent, and the same id cannot
/// is last-write-wins.
#[test]
fn same_id_operations_apply_in_order() {
    for (name, config) in configs() {
        let dir = tempfile::tempdir().unwrap();
        let core = open(dir.path(), &config).unwrap();
        seed(&core);
        let results = core
            .write_batch(vec![
                del("swap"),
                put("swap", "swap zebra reborn"),
                put("late", "late zebra"),
                del("late"),
                del("late"),
                del("keep"),
                del("keep"),
            ])
            .unwrap();
        assert_eq!(
            results,
            [
                BatchResult::Delete { existed: true },
                BatchResult::Memorize { chunks: 1 },
                BatchResult::Memorize { chunks: 1 },
                BatchResult::Delete { existed: true },
                BatchResult::Delete { existed: false },
                BatchResult::Delete { existed: true },
                BatchResult::Delete { existed: false },
            ],
            "{name}"
        );
        let check = |core: &WarmCore| {
            assert!(core.get_document("late").unwrap().is_none(), "{name}");
            assert!(core.get_document("keep").unwrap().is_none(), "{name}");
            assert!(
                core.get_document("swap")
                    .unwrap()
                    .is_some_and(|d| d.text.as_deref().is_some_and(|t| t.contains("reborn"))),
                "{name}"
            );
            let hits = core.search_documents("zebra", &Default::default()).unwrap();
            let mut ids: Vec<_> = hits.iter().map(|h| h.document_id.as_str()).collect();
            ids.sort();
            assert_eq!(ids, ["edit", "gone", "swap"], "{name}");
        };
        check(&core);
        let before = snapshot(&core);
        drop(core);
        let core = open(dir.path(), &config).unwrap();
        check(&core);
        assert_eq!(snapshot(&core), before, "{name}: replay differs from live");

        // The same id memorized twice: the last write wins, each op reports its own result.
        let results = core
            .write_batch(vec![
                put("late", "late zebra first"),
                put("late", "late zebra second two words"),
                put("keep", "keep zebra back"),
                del("keep"),
                put("keep", "keep zebra final"),
            ])
            .unwrap();
        assert_eq!(results.len(), 5, "{name}");
        let text = |core: &WarmCore, id: &str| core.get_document(id).unwrap().unwrap().text;
        assert_eq!(
            text(&core, "late").as_deref(),
            Some("late zebra second two words")
        );
        assert_eq!(text(&core, "keep").as_deref(), Some("keep zebra final"));
        let before = snapshot(&core);
        drop(core);
        let core = open(dir.path(), &config).unwrap();
        assert_eq!(snapshot(&core), before, "{name}: replay differs from live");
        assert_eq!(
            text(&core, "late").as_deref(),
            Some("late zebra second two words")
        );
    }
}

#[test]
fn an_invalid_operation_applies_nothing() {
    for (name, config) in configs() {
        let dir = tempfile::tempdir().unwrap();
        let core = open(dir.path(), &config).unwrap();
        seed(&core);
        let before = snapshot(&core);
        let revision = core.revision();
        let invalid_document = {
            let mut ops = batch();
            ops.push(put("", "no id"));
            ops
        };
        let invalid_delete = {
            let mut ops = batch();
            ops.push(del(""));
            ops
        };
        let invalid_embedding = {
            let mut ops = batch();
            let mut bad = input("bad", "bad zebra");
            bad.embeddings = Some(vec![vec![f64::NAN; 3]]);
            ops.push(BatchOp::Memorize { document: bad });
            ops
        };
        for (case, ops) in [
            ("empty id", invalid_document),
            ("empty delete id", invalid_delete),
            ("bad embedding", invalid_embedding),
            ("empty batch", vec![]),
        ] {
            assert!(core.write_batch(ops).is_err(), "{name}: {case} accepted");
            assert_eq!(snapshot(&core), before, "{name}: {case} leaked");
            assert_eq!(core.revision(), revision, "{name}: {case} moved revision");
        }
        drop(core);
        let core = open(dir.path(), &config).unwrap();
        assert_eq!(snapshot(&core), before, "{name}: journal changed");
    }
}

#[test]
fn the_batch_size_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path(), &plain()).unwrap();
    for count in [4095usize, 4096, 4097] {
        let ops = (0..count).map(|i| del(&format!("absent-{i}"))).collect();
        let result = core.write_batch(ops);
        if count <= 4096 {
            assert_eq!(result.unwrap().len(), count);
        } else {
            let error = result.unwrap_err();
            assert!(error.is::<BatchTooLarge>(), "{error:#}");
        }
    }
}

/// The protocol, not luck: take the exact bytes the writer appends for one batch and replay
/// every prefix of them into the journal of the store as it was before the batch. A store
/// opened as a reader or as the writer holds the whole batch or none of it, and an
/// incomplete prefix is never a commit.
#[test]
fn a_crash_at_every_byte_of_the_batch_frame_keeps_all_or_none() {
    use std::io::{Seek, SeekFrom, Write};
    for (name, config) in configs() {
        let dir = tempfile::tempdir().unwrap();
        let before = dir.path().join("before");
        let after = dir.path().join("after");
        {
            let core = open(&before, &config).unwrap();
            seed(&core);
        }
        copy_dir(&before, &after);
        {
            let core = open(&after, &config).unwrap();
            let results = core.write_batch(batch()).unwrap();
            assert_eq!(results.len(), 7);
        }
        let old = std::fs::read(segment(&before)).unwrap();
        let new = std::fs::read(segment(&after)).unwrap();
        let start = (0..old.len().min(new.len()))
            .find(|&i| old[i] != new[i])
            .expect("the batch wrote a frame");
        let compressed =
            u32::from_le_bytes(new[start + 8..start + 12].try_into().unwrap()) as usize;
        let frame = &new[start..start + 16 + compressed];

        let reference = |store: &Path| snapshot(&open_reader(store, &config).unwrap());
        let none = reference(&before);
        let all = {
            let probe = dir.path().join("probe");
            copy_dir(&after, &probe);
            reference(&probe)
        };
        assert_ne!(none, all, "{name}: the batch changed nothing");

        for cut in 0..=frame.len() {
            let store = dir.path().join(format!("cut-{cut}"));
            copy_dir(&before, &store);
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .open(segment(&store))
                .unwrap();
            file.seek(SeekFrom::Start(start as u64)).unwrap();
            file.write_all(&frame[..cut]).unwrap();
            drop(file);
            if cut == frame.len() {
                // The writer publishes the committed end once the frame is durable; readers
                // never scan past it. A shorter prefix is a crash before that publication.
                let published = segment(&after).with_file_name("committed_end");
                if published.exists() {
                    std::fs::copy(&published, segment(&store).with_file_name("committed_end"))
                        .unwrap();
                }
            }
            let expected = if cut == frame.len() { &all } else { &none };
            assert_eq!(
                &reference(&store),
                expected,
                "{name}: reader at a {cut}/{} byte prefix",
                frame.len()
            );
            // Reopening as the writer rewrites the tail and is the slow step: every byte near
            // the header and the end, then a stride across the payload.
            if cut > 20 && cut + 20 < frame.len() && cut % 7 != 0 {
                continue;
            }
            let writer = open(&store, &config).unwrap();
            assert_eq!(
                &snapshot(&writer),
                expected,
                "{name}: writer at a {cut}/{} byte prefix",
                frame.len()
            );
        }
    }
}

/// A reader refreshing while the writer commits batches never sees half of one. Batch `n`
/// memorizes `a-n` and `b-n` and deletes `a-(n-1)` and `b-(n-1)`, so every state a reader
/// can hold has exactly one complete pair.
#[test]
fn a_concurrent_reader_never_observes_part_of_a_batch() {
    use std::sync::atomic::{AtomicBool, Ordering};
    for (name, config) in configs() {
        let dir = tempfile::tempdir().unwrap();
        let writer = open(dir.path(), &config).unwrap();
        writer
            .write_batch(vec![put("a-0", "pair zebra 0"), put("b-0", "pair zebra 0")])
            .unwrap();
        let mut reader = open_reader(dir.path(), &config).unwrap();
        let done = AtomicBool::new(false);
        let rounds = 60;
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for n in 1..=rounds {
                    writer
                        .write_batch(vec![
                            put(&format!("a-{n}"), &format!("pair zebra {n}")),
                            del(&format!("a-{}", n - 1)),
                            put(&format!("b-{n}"), &format!("pair zebra {n}")),
                            del(&format!("b-{}", n - 1)),
                        ])
                        .unwrap();
                }
                done.store(true, Ordering::Release);
            });
            let mut seen = 0;
            let mut last = 0;
            while !done.load(Ordering::Acquire) || seen < 3 {
                reader.refresh().unwrap();
                let hits = reader
                    .search_documents("zebra", &Default::default())
                    .unwrap();
                let mut ids: Vec<_> = hits.iter().map(|h| h.document_id.clone()).collect();
                ids.sort();
                let n: u32 = ids[0].trim_start_matches("a-").parse().unwrap();
                assert_eq!(
                    ids,
                    [format!("a-{n}"), format!("b-{n}")],
                    "{name}: a partial batch was visible"
                );
                assert_eq!(reader.document_count(), 2, "{name}");
                assert!(n >= last, "{name}: the reader went backwards");
                last = n;
                seen += 1;
            }
        });
        reader.refresh().unwrap();
        assert!(reader
            .get_document(&format!("a-{rounds}"))
            .unwrap()
            .is_some());
    }
}

#[test]
fn a_reader_cannot_write_a_batch() {
    let dir = tempfile::tempdir().unwrap();
    drop(open(dir.path(), &plain()).unwrap());
    let reader = open_reader(dir.path(), &plain()).unwrap();
    let error = reader.write_batch(vec![del("x")]).unwrap_err();
    assert!(error.downcast_ref::<ReadOnlyError>().is_some(), "{error:#}");
}

/// A reader in another process polling while the writer commits 1000 batches never sees part
/// of one: batch `n` writes `K` documents tagged `n`, and every tag it can see is whole.
const BATCHES: usize = 1000;
const K: usize = 4;

#[test]
fn reader_process_role() {
    let Ok(dir) = std::env::var("WARM_BATCH_READER_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let done = dir.with_extension("done");
    let mut reader = open_reader(&dir, &plain()).unwrap();
    let mut polls = 0usize;
    let mut last_tags = 0usize;
    loop {
        let finished = done.exists();
        reader.refresh().unwrap();
        let mut per_tag = std::collections::BTreeMap::<String, usize>::new();
        let mut after = None;
        loop {
            let page = reader.list_document_info(after.as_deref(), 1000).unwrap();
            let Some(last) = page.last() else { break };
            after = Some(format!("{}\0", last.id));
            for info in &page {
                *per_tag
                    .entry(info.id.split('-').next().unwrap().to_string())
                    .or_default() += 1;
            }
        }
        for (tag, count) in &per_tag {
            assert_eq!(*count, K, "tag {tag} partially visible after {polls} polls");
        }
        assert!(per_tag.len() >= last_tags, "the reader went backwards");
        last_tags = per_tag.len();
        polls += 1;
        if finished {
            break;
        }
    }
    println!("RESULT reader tags={last_tags} polls={polls}");
}

#[test]
fn a_reader_process_never_observes_part_of_a_batch_across_1000_batches() {
    use std::io::{BufRead, BufReader};
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store");
    let writer = open(&store, &plain()).unwrap();
    // The reader opens an existing store; start it with the first batch committed.
    let batch = |n: usize| -> Vec<BatchOp> {
        (0..K)
            .map(|i| put(&format!("t{n}-{i}"), &format!("tag {n} part {i} zebra")))
            .collect()
    };
    writer.write_batch(batch(0)).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "reader_process_role",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("WARM_BATCH_READER_DIR", &store)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    for n in 1..BATCHES {
        writer.write_batch(batch(n)).unwrap();
    }
    std::fs::write(store.with_extension("done"), b"").unwrap();
    let mut report = None;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let line = line.unwrap();
        if let Some(at) = line.find("RESULT reader") {
            report = Some(line[at..].to_string());
        }
    }
    assert!(child.wait().unwrap().success(), "the reader process failed");
    let report = report.expect("the reader reported");
    assert!(report.contains(&format!("tags={BATCHES}")), "{report}");
}
