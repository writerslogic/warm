// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
// Separate test target: re-executes this binary by test name to run writers and readers as
// real processes.

//! One writer plus any number of read-only readers (`docs/reference/concurrency.md`).
//!
//! A reader takes no lock, so it never blocks the writer and the writer never blocks it. Its
//! consistency guarantee is the one tested here: every state it observes is a state the
//! writer committed, never a mix of two.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use writerslogic_warm::core::WarmConfig;
use writerslogic_warm::{ReadOnlyError, StoreLockedError, WarmCore};

const DIM: u32 = 256;

fn config() -> Option<WarmConfig> {
    Some(WarmConfig::unencrypted())
}

fn writer(dir: &Path) -> anyhow::Result<WarmCore> {
    WarmCore::new(DIM, Some(dir.display().to_string()), config())
}

fn reader(dir: &Path) -> anyhow::Result<WarmCore> {
    WarmCore::open_read_only(DIM, Some(dir.display().to_string()), config())
}

fn doc(id: &str, text: &str) -> writerslogic_warm::core::documents::DocumentInput {
    serde_json::from_value(serde_json::json!({ "id": id, "text": text })).unwrap()
}

/// One commit of two documents that share a counter: a reader sees both or neither.
fn commit_pair(core: &WarmCore, n: u32) -> anyhow::Result<()> {
    core.memorize_documents(vec![
        doc(
            &format!("a-{n}"),
            &format!("alpha counter {n} stored together"),
        ),
        doc(
            &format!("b-{n}"),
            &format!("beta counter {n} stored together"),
        ),
    ])?;
    Ok(())
}

/// The highest counter visible, after checking the visible set is exactly the pairs
/// `1..=max`: no half pair and no gap.
fn visible_counter(core: &WarmCore) -> anyhow::Result<u32> {
    let count = core.document_count();
    anyhow::ensure!(
        count & 1 == 0,
        "odd document count {count}: half a transaction"
    );
    let max = (count / 2) as u32;
    for n in 1..=max {
        for side in ["a", "b"] {
            anyhow::ensure!(
                core.get_document(&format!("{side}-{n}"))?.is_some(),
                "{side}-{n} missing with {count} documents visible"
            );
        }
    }
    Ok(max)
}

fn child(role: &str, dir: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "process_role", "--nocapture", "--test-threads=1"])
        .env("WARM_SHARED_ROLE", role)
        .env("WARM_SHARED_DIR", dir)
        .stdout(Stdio::piped())
        .stdin(Stdio::piped());
    command
}

fn lines(child: &mut Child) -> BufReader<std::process::ChildStdout> {
    BufReader::new(child.stdout.take().unwrap())
}

/// Reads stdout until a line starting with `RESULT <tag>`; returns the rest of it.
fn wait_for(out: &mut BufReader<std::process::ChildStdout>, tag: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(120);
    let prefix = format!("RESULT {tag}");
    let mut line = String::new();
    while Instant::now() < deadline {
        line.clear();
        if out.read_line(&mut line).unwrap() == 0 {
            panic!("child exited before reporting {tag}");
        }
        // The harness prints `test process_role ... ` without a newline before the test runs.
        if let Some(at) = line.find(&prefix) {
            return line[at + prefix.len()..].trim().to_string();
        }
    }
    panic!("timed out waiting for {tag}");
}

fn say(text: &str) {
    println!("RESULT {text}");
    std::io::stdout().flush().unwrap();
}

/// Child entry point; does nothing unless a parent set the role.
#[test]
fn process_role() {
    let Ok(role) = std::env::var("WARM_SHARED_ROLE") else {
        return;
    };
    let dir = PathBuf::from(std::env::var("WARM_SHARED_DIR").unwrap());
    let stop = dir.with_extension("stop");
    match role.as_str() {
        // Commits pairs until told to stop (or killed).
        "writer-loop" => {
            let core = writer(&dir).unwrap();
            say("ready");
            let mut n = 0;
            while !stop.exists() {
                n += 1;
                commit_pair(&core, n).unwrap();
            }
            say(&format!("done {n}"));
        }
        // Holds the store until the parent writes to stdin.
        "hold" => {
            let _core = writer(&dir).unwrap();
            say("ready");
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
        }
        // Commits, closes, and then stays alive: the lock must be gone before the process is.
        "close-then-linger" => {
            let core = writer(&dir).unwrap();
            commit_pair(&core, 1).unwrap();
            core.close().unwrap();
            say("closed");
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
        }
        // Refreshes in a loop and checks every state it observes.
        "reader-loop" => {
            let mut core = reader(&dir).unwrap();
            let mut seen = visible_counter(&core).unwrap_or_else(|e| {
                say(&format!("torn {e:#}"));
                std::process::exit(1)
            });
            let mut refreshes = 0u32;
            say("ready");
            while !stop.exists() {
                if core.refresh().unwrap() {
                    refreshes += 1;
                }
                match visible_counter(&core) {
                    Ok(max) => {
                        assert!(max >= seen, "state went backwards: {max} < {seen}");
                        seen = max;
                    }
                    Err(error) => {
                        say(&format!("torn {error:#}"));
                        std::process::exit(1);
                    }
                }
            }
            say(&format!("ok {seen} {refreshes}"));
        }
        other => panic!("unknown role {other}"),
    }
}

fn tree_digest(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if name == ".hms.readers" {
                continue;
            }
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_path_buf();
                out.push((rel, std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn snapshots(dir: &Path) -> usize {
    std::fs::read_dir(dir.join(".hms.readers"))
        .map(|d| d.count())
        .unwrap_or(0)
}

#[test]
fn readers_run_beside_a_writer_and_see_only_whole_transactions() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store");
    let mut writer_child = child("writer-loop", &store).spawn().unwrap();
    let mut writer_out = lines(&mut writer_child);
    wait_for(&mut writer_out, "ready");

    let mut readers: Vec<_> = (0..3)
        .map(|_| {
            let mut c = child("reader-loop", &store).spawn().unwrap();
            let mut out = lines(&mut c);
            wait_for(&mut out, "ready");
            (c, out)
        })
        .collect();
    // The writer keeps committing while the readers run.
    std::thread::sleep(Duration::from_secs(3));
    std::fs::write(store.with_extension("stop"), b"").unwrap();

    let done = wait_for(&mut writer_out, "done");
    let committed: u32 = done.parse().unwrap();
    assert!(committed > 5, "the writer was starved: {committed} commits");
    for (c, out) in &mut readers {
        let report = wait_for(out, "ok");
        let mut fields = report.split_whitespace();
        let seen: u32 = fields.next().unwrap().parse().unwrap();
        let refreshes: u32 = fields.next().unwrap().parse().unwrap();
        assert!(seen > 0 && refreshes > 0, "reader never advanced: {report}");
        assert!(c.wait().unwrap().success());
    }
    assert!(writer_child.wait().unwrap().success());
    // A final reader sees everything the writer committed.
    let last = reader(&store).unwrap();
    assert_eq!(visible_counter(&last).unwrap(), committed);
}

#[test]
fn a_reader_open_sees_a_consistent_state_after_the_writer_is_killed_mid_write() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store");
    let mut writer_child = child("writer-loop", &store).spawn().unwrap();
    let mut out = lines(&mut writer_child);
    wait_for(&mut out, "ready");
    // Kill while it is committing, with a reader attached.
    let observer = reader(&store).unwrap();
    let before = visible_counter(&observer).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    writer_child.kill().unwrap();
    writer_child.wait().unwrap();

    let before_open = tree_digest(&store);
    let after_kill = reader(&store).unwrap();
    let seen = visible_counter(&after_kill).unwrap();
    assert!(seen >= before);
    drop(after_kill);
    assert!(
        tree_digest(&store) == before_open,
        "a read-only open changed the shared store"
    );

    // The next writer recovers the torn tail, and a reader picks the new commit up.
    let recovered = writer(&store).unwrap();
    commit_pair(&recovered, seen + 1).unwrap();
    let mut observer = observer;
    assert!(observer.refresh().unwrap());
    assert_eq!(visible_counter(&observer).unwrap(), seen + 1);
}

#[test]
fn a_refresh_sees_new_commits_and_the_old_state_stays_consistent_until_then() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store");
    let w = writer(&store).unwrap();
    commit_pair(&w, 1).unwrap();
    let mut r = reader(&store).unwrap();
    assert_eq!(visible_counter(&r).unwrap(), 1);
    assert!(!r.has_newer_commits().unwrap());
    assert!(!r.refresh().unwrap());

    commit_pair(&w, 2).unwrap();
    assert_eq!(
        visible_counter(&r).unwrap(),
        1,
        "a snapshot does not move on its own"
    );
    assert!(r.has_newer_commits().unwrap());
    assert!(r.refresh().unwrap());
    assert_eq!(visible_counter(&r).unwrap(), 2);

    // A compaction publishes a new generation; the reader follows it.
    w.compact().unwrap();
    commit_pair(&w, 3).unwrap();
    assert!(r.refresh().unwrap());
    assert_eq!(visible_counter(&r).unwrap(), 3);
    assert_eq!(snapshots(&store), 1, "refresh left the old snapshot behind");
    drop(r);
    assert_eq!(
        snapshots(&store),
        0,
        "closing a reader left its snapshot behind"
    );
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let name = entry.file_name();
        if name == ".hms.lock" || name == ".hms.readers" {
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

/// The journal segment a small store writes into.
fn segment(store: &Path) -> PathBuf {
    let arena = store.join("vectors_data.bin");
    let live = match std::fs::read_to_string(arena.join("CURRENT")) {
        Ok(name) => arena.join(name),
        Err(_) => arena,
    };
    live.join("seg_0.bin")
}

/// The protocol, not luck: take the exact bytes a writer appends for one commit and replay
/// them into a store prefix by prefix, the way a reader can catch a frame mid-write. Every
/// incomplete prefix reads as the old state, the complete frame as the new one.
#[test]
fn a_commit_caught_mid_write_is_invisible_until_it_is_whole() {
    use std::io::{Seek, SeekFrom};
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store");
    let shadow = dir.path().join("shadow");
    {
        let w = writer(&store).unwrap();
        commit_pair(&w, 1).unwrap();
    }
    copy_dir(&store, &shadow);
    let shadow_writer = writer(&shadow).unwrap();
    commit_pair(&shadow_writer, 2).unwrap();

    let old = std::fs::read(segment(&store)).unwrap();
    let new = std::fs::read(segment(&shadow)).unwrap();
    let start = (0..old.len().min(new.len()))
        .find(|&i| old[i] != new[i])
        .expect("the second commit wrote a frame");
    let compressed = u32::from_le_bytes(new[start + 8..start + 12].try_into().unwrap()) as usize;
    let frame = &new[start..start + 16 + compressed];

    let write_prefix = |len: usize| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(segment(&store))
            .unwrap();
        file.seek(SeekFrom::Start(start as u64)).unwrap();
        file.write_all(&frame[..len]).unwrap();
    };
    let mut cuts = vec![0, 1, 4, 15, 16, 17, frame.len() / 2, frame.len() - 1];
    cuts.dedup();
    let mut held = None;
    for cut in cuts {
        write_prefix(cut);
        let r = reader(&store).unwrap();
        assert_eq!(
            visible_counter(&r).unwrap(),
            1,
            "a {cut}-byte prefix leaked"
        );
        assert!(
            !r.has_newer_commits().unwrap(),
            "a torn frame counted as a commit"
        );
        held = Some(r);
    }
    let mut r = held.unwrap();
    write_prefix(frame.len());
    assert!(r.has_newer_commits().unwrap());
    assert!(r.refresh().unwrap());
    assert_eq!(visible_counter(&r).unwrap(), 2);
}

/// Dense vectors live in a sub-store beside the journal; a reader copies it and finds them.
#[test]
fn a_reader_copies_the_dense_vector_sub_store_and_settles_when_nothing_changes() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store");
    let w = writer(&store).unwrap();
    let vector = |seed: f32| -> Vec<f32> { (0..32).map(|i| (i as f32 + seed).sin()).collect() };
    for i in 0..40 {
        w.memorize_vector(format!("v-{i}"), &vector(i as f32))
            .unwrap();
    }
    assert!(store.join("dense-vectors").is_dir());
    let mut r = reader(&store).unwrap();
    let hits = r.query_vector(&vector(7.0), 1).unwrap();
    assert_eq!(hits[0].id, "v-7");
    // Nothing changed: the listing the reader took must compare equal, not drift.
    assert!(!r.has_newer_commits().unwrap());
    assert!(!r.refresh().unwrap());

    w.memorize_vector("v-late".into(), &vector(99.0)).unwrap();
    assert!(r.refresh().unwrap());
    assert_eq!(r.query_vector(&vector(99.0), 1).unwrap()[0].id, "v-late");
}

#[test]
fn a_reader_cannot_mutate_and_cannot_create_a_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store");
    let w = writer(&store).unwrap();
    commit_pair(&w, 1).unwrap();
    let r = reader(&store).unwrap();

    let error = r.memorize_document(doc("c-1", "not allowed")).unwrap_err();
    assert!(error.downcast_ref::<ReadOnlyError>().is_some(), "{error:#}");
    assert!(r
        .compact()
        .unwrap_err()
        .downcast_ref::<ReadOnlyError>()
        .is_some());
    assert_eq!(visible_counter(&r).unwrap(), 1);
    assert!(r.get_document("c-1").unwrap().is_none());

    let missing = dir.path().join("missing");
    assert!(reader(&missing).is_err());
    assert!(!missing.join("store.json").exists());
}

#[test]
fn a_second_writer_is_refused_naming_the_holder_and_readers_are_not() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store");
    drop(writer(&store).unwrap());
    let mut holder = child("hold", &store).spawn().unwrap();
    let mut out = lines(&mut holder);
    wait_for(&mut out, "ready");

    let error = writer(&store)
        .err()
        .expect("a second writer must be refused");
    let locked = error
        .downcast_ref::<StoreLockedError>()
        .unwrap_or_else(|| panic!("not a lock error: {error:#}"));
    // Windows denies reads of a locked file, so only Unix can name the holder.
    match locked.holder_pid {
        Some(pid) => {
            assert_eq!(pid, holder.id());
            assert!(error.to_string().contains(&format!("pid {pid}")));
        }
        None => assert!(error.to_string().contains("holder unknown")),
    }
    #[cfg(unix)]
    assert!(locked.holder_pid.is_some());
    assert!(error.to_string().contains("already open"));

    // The holder does not keep readers out.
    assert_eq!(visible_counter(&reader(&store).unwrap()).unwrap(), 0);

    holder.stdin.take().unwrap().write_all(b"\n").unwrap();
    assert!(holder.wait().unwrap().success());
    drop(writer(&store).unwrap());
}

#[test]
fn close_releases_the_lock_while_the_process_lives() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store");
    let mut c = child("close-then-linger", &store).spawn().unwrap();
    let mut out = lines(&mut c);
    wait_for(&mut out, "closed");
    // The child is still running; only close() can have released the lock.
    let next = writer(&store).expect("close must release the lock");
    assert_eq!(visible_counter(&next).unwrap(), 1);
    drop(next);
    c.stdin.take().unwrap().write_all(b"\n").unwrap();
    assert!(c.wait().unwrap().success());
}

#[cfg(feature = "security")]
#[test]
fn a_reader_opens_an_encrypted_store_with_its_key() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store");
    {
        let w = WarmCore::new(DIM, Some(store.display().to_string()), None).unwrap();
        commit_pair(&w, 1).unwrap();
        assert!(store.join("master.key").exists());
        let r = WarmCore::open_read_only(DIM, Some(store.display().to_string()), None).unwrap();
        assert_eq!(visible_counter(&r).unwrap(), 1);
        // The key is read from the store; the snapshot holds no copy of it.
        let copies = files_named(&store.join(".hms.readers"), "master.key");
        assert!(copies.is_empty(), "key copied into a snapshot: {copies:?}");
        assert!(!files_named(&store.join(".hms.readers"), "key.check").is_empty());
    }
}

#[cfg(feature = "security")]
fn files_named(dir: &Path, name: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_named(&path, name));
        } else if entry.file_name() == name {
            found.push(path);
        }
    }
    found
}
