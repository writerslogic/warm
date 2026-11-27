// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! `write_batch` against a loop of single calls: wall time and journal frames (each frame is
//! one durable sync) for N mixed memorize/delete operations.
//! `cargo run --release --example write_batch_bench`

use std::path::Path;
use std::time::Instant;
use writerslogic_warm::core::documents::DocumentInput;
use writerslogic_warm::core::WarmConfig;
use writerslogic_warm::{BatchOp, WarmCore};

fn doc(id: &str) -> DocumentInput {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "text": format!("document {id} with a short body of text for the benchmark"),
    }))
    .unwrap()
}

/// Memorize on even positions, delete the previous document on odd ones.
fn ops(n: usize) -> Vec<BatchOp> {
    (0..n)
        .map(|i| {
            if i % 2 == 0 {
                BatchOp::Memorize {
                    document: doc(&format!("b{i}")),
                }
            } else {
                BatchOp::Delete {
                    id: format!("b{}", i - 1),
                }
            }
        })
        .collect()
}

fn frames(store: &Path) -> usize {
    let arena = store.join("vectors_data.bin");
    let live = match std::fs::read_to_string(arena.join("CURRENT")) {
        Ok(name) => arena.join(name),
        Err(_) => arena,
    };
    let bytes = std::fs::read(live.join("seg_0.bin")).unwrap();
    let (mut offset, mut count) = (0, 0);
    while offset + 16 <= bytes.len() {
        let comp = u32::from_le_bytes(bytes[offset + 8..offset + 12].try_into().unwrap()) as usize;
        if comp == 0 {
            break;
        }
        offset += 16 + comp;
        count += 1;
    }
    count
}

fn open(dir: &Path) -> WarmCore {
    WarmCore::new(
        256,
        Some(dir.display().to_string()),
        Some(WarmConfig::unencrypted()),
    )
    .unwrap()
}

fn main() {
    println!("{:>6} {:>14} {:>10} {:>8}", "N", "mode", "ms", "frames");
    for n in [1usize, 100, 1000] {
        for mode in ["loop", "batch"] {
            let mut times = Vec::new();
            let mut frame_count = 0;
            for _ in 0..3 {
                let dir = tempfile::tempdir().unwrap();
                let core = open(dir.path());
                // One committed frame first so the timed section excludes store creation.
                core.memorize_document(doc("warmup")).unwrap();
                let before = frames(dir.path());
                let work = ops(n);
                let start = Instant::now();
                if mode == "batch" {
                    core.write_batch(work).unwrap();
                } else {
                    for op in work {
                        match op {
                            BatchOp::Memorize { document } => {
                                core.memorize_document(document).unwrap();
                            }
                            BatchOp::Delete { id } => {
                                core.delete_document(&id).unwrap();
                            }
                        }
                    }
                }
                times.push(start.elapsed().as_secs_f64() * 1e3);
                frame_count = frames(dir.path()) - before;
            }
            times.sort_by(|a, b| a.partial_cmp(b).unwrap());
            println!("{n:>6} {mode:>14} {:>10.1} {frame_count:>8}", times[1]);
        }
    }
}
