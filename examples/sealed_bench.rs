// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Microbenchmark of the sealed block container: seal and open throughput, container open
//! time at 20 MiB and 1.4 GiB, and per-block overhead.
//!
//! `cargo run --release --features security --example sealed_bench -- [DIR]`

#[cfg(not(feature = "security"))]
fn main() {
    eprintln!("build with --features security");
}

#[cfg(feature = "security")]
fn main() -> anyhow::Result<()> {
    use std::time::Instant;
    use writerslogic_warm::core::sealed::{KeySource, Mode, SealedStore, StoreOptions};

    const MIB: usize = 1 << 20;
    const BLOCK: usize = 64 * 1024;
    let base = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("warm-sealed-bench-{}", std::process::id()));
    std::fs::create_dir_all(&base)?;
    let block: Vec<u8> = (0..BLOCK)
        .map(|i| (i.wrapping_mul(2654435761) >> 7) as u8)
        .collect();

    for (label, mode) in [("aes-256-gcm", Mode::Aes256Gcm), ("plaintext", Mode::Plain)] {
        for (name, total) in [("20MiB", 20 * MIB), ("1.4GiB", 1434 * MIB)] {
            let dir = base.join(format!("{label}-{name}"));
            let store = SealedStore::create(
                &dir,
                StoreOptions {
                    mode: Some(mode),
                    source: (mode == Mode::Aes256Gcm)
                        .then(|| KeySource::file(base.join(format!("{label}.key")))),
                },
            )?;
            let blocks = total / BLOCK;
            let mut writer = store.create_segment(BLOCK as u32)?;
            let t = Instant::now();
            for _ in 0..blocks {
                writer.append_block(&block)?;
            }
            let seal = t.elapsed();
            let t = Instant::now();
            writer.commit()?;
            let commit = t.elapsed();
            let id = writer.segment_id();
            drop(writer);

            let mut opens = Vec::new();
            for _ in 0..5 {
                let t = Instant::now();
                let reader = store.open_segment_reader(id)?;
                opens.push(t.elapsed());
                drop(reader);
            }
            opens.sort();
            let reader = store.open_segment_reader(id)?;
            reader.set_cache_bytes(0);
            let t = Instant::now();
            for i in 0..reader.block_count() {
                std::hint::black_box(reader.read_block(i)?);
            }
            let open = t.elapsed();
            let mibs =
                |d: std::time::Duration| (blocks * BLOCK) as f64 / MIB as f64 / d.as_secs_f64();
            let hdr: u64 = std::fs::read_dir(&dir)?
                .filter_map(Result::ok)
                .filter(|e| e.path().extension().is_some_and(|x| x == "hdr"))
                .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
                .sum();
            println!(
                "{label} {name}: blocks={blocks} seal={:.0} MiB/s commit={:.1}ms open_segment={:.3}ms (median of 5) read+decrypt={:.0} MiB/s overhead={:.2} B/block ({} B header, 0 B in data file)",
                mibs(seal),
                commit.as_secs_f64() * 1e3,
                opens[2].as_secs_f64() * 1e3,
                mibs(open),
                hdr as f64 / blocks as f64,
                hdr,
            );
            drop(reader);
            drop(store);
            std::fs::remove_dir_all(&dir)?;
        }
    }
    let _ = std::fs::remove_dir_all(&base);
    Ok(())
}
