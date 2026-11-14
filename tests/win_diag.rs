#![cfg(feature = "failpoints")]
mod erase_common;
use erase_common::*;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

const OPS: usize = 14;

#[test]
fn diag_child() {
    let Ok(dir) = std::env::var("WARM_CRASH_CHILD_DIR") else {
        return;
    };
    let memory = open(Path::new(&dir)).unwrap();
    for (n, op) in plan(OPS).into_iter().enumerate() {
        apply(&memory, op).unwrap();
        println!("ACK {n}");
    }
}

fn run(dir: &Path, fp: Option<&str>, trace: Option<&Path>) -> (Vec<u128>, Option<i32>, String, u128) {
    let t0 = Instant::now();
    let mut c = Command::new(std::env::current_exe().unwrap());
    c.args(["--exact", "diag_child", "--nocapture", "--test-threads=1"])
        .env("WARM_CRASH_CHILD_DIR", dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(s) = fp {
        c.env("WARM_FAILPOINT", s);
    }
    if let Some(p) = trace {
        c.env("WARM_FAILPOINT_TRACE", p);
    }
    let mut child = c.spawn().unwrap();
    let spawn = t0.elapsed().as_millis();
    let mut acks = Vec::new();
    for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
        if line.contains("ACK ") {
            acks.push(t0.elapsed().as_millis());
        }
    }
    let mut err = String::new();
    let _ = child.stderr.take().unwrap().read_to_string(&mut err);
    let code = child.wait().unwrap().code();
    (acks, code, err, spawn)
}

fn free_gb(p: &Path) -> String {
    let out = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "(Get-PSDrive -Name ('{}'.Substring(0,1))).Free / 1MB",
                p.display()
            ),
        ])
        .output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

#[test]
fn win_diag() {
    let work = tempfile::tempdir().unwrap();
    let w = work.path();
    println!("DIAG work={} free_mb_before={}", w.display(), free_gb(w));
    let trace = w.join("trace");
    let t = Instant::now();
    let (acks, code, err, spawn) = run(&w.join("count"), None, Some(&trace));
    println!(
        "DIAG full run: {} ms, spawn {} ms, acks {:?}, code {:?}, store bytes {} files {}",
        t.elapsed().as_millis(),
        spawn,
        acks,
        code,
        tree_bytes(&w.join("count")),
        files(&w.join("count")).len()
    );
    println!("DIAG stderr: {}", err.lines().take(5).collect::<Vec<_>>().join(" | "));
    let rows: Vec<(String, u128)> = std::fs::read_to_string(&trace)
        .unwrap()
        .lines()
        .map(|l| {
            let (n, t) = l.split_once(' ').unwrap();
            (n.to_string(), t.parse().unwrap())
        })
        .collect();
    let mut agg: BTreeMap<String, (u64, u128)> = BTreeMap::new();
    for (i, (n, t0)) in rows.iter().enumerate() {
        let next = rows.get(i + 1).map_or(*t0, |r| r.1);
        let e = agg.entry(n.clone()).or_default();
        e.0 += 1;
        e.1 += next - t0;
    }
    println!("DIAG points {}", rows.len());
    for (n, (c, us)) in &agg {
        println!("DIAG point {n}: count {c}, time-until-next-hit {} ms", us / 1000);
    }
    let mut slow: Vec<(u128, usize)> = rows
        .windows(2)
        .enumerate()
        .map(|(i, p)| (p[1].1 - p[0].1, i))
        .collect();
    slow.sort();
    for (us, i) in slow.iter().rev().take(8) {
        println!("DIAG slowest gap {} ms after #{} {}", us / 1000, i + 1, rows[*i].0);
    }
    let total = rows.len();
    let ops = plan(OPS);
    for k in [1, total / 4, total / 2, 3 * total / 4, total] {
        let dir = w.join(format!("run{k}"));
        let t = Instant::now();
        let (acks, code, err, spawn) = run(&dir, Some(&format!("exit@{k}")), None);
        let child_ms = t.elapsed().as_millis();
        let bytes = tree_bytes(&dir);
        let t = Instant::now();
        check_reopened(&dir, &ops[..acks.len()], &ops[acks.len()..(acks.len() + 1).min(ops.len())], "diag");
        let verify_ms = t.elapsed().as_millis();
        let t = Instant::now();
        let m = open(&dir).unwrap();
        apply(&m, Op::Remember(OPS + 1)).unwrap();
        drop(m);
        let again_ms = t.elapsed().as_millis();
        let t = Instant::now();
        std::fs::remove_dir_all(&dir).unwrap();
        println!(
            "DIAG exit@{k}: code {:?} spawn {spawn} ms child {child_ms} ms (last ack at {:?}) store {bytes} B; verify(reopen+scan) {verify_ms} ms; reopen+write {again_ms} ms; rmdir {} ms; stderr {}; free_mb {}",
            code,
            acks.last(),
            t.elapsed().as_millis(),
            err.lines().take(3).collect::<Vec<_>>().join(" | "),
            free_gb(w)
        );
    }
}
