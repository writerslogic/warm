// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Provider round trip across processes: `create DIR`, `open DIR`, `delete DIR`, `open DIR`
//! (which must now fail). Uses `KeySource::auto`.
//!
//! `cargo run --features keychain --example sealed_keyprobe -- create DIR`

#[cfg(not(feature = "keychain"))]
fn main() {
    eprintln!("build with --features keychain");
}

#[cfg(feature = "keychain")]
fn main() -> anyhow::Result<()> {
    use writerslogic_warm::core::sealed::{KeySource, Mode, SealedStore, StoreOptions};
    let args: Vec<String> = std::env::args().collect();
    let (cmd, dir) = (args[1].as_str(), std::path::PathBuf::from(&args[2]));
    let source = KeySource::auto(&dir, None);
    let kind = source.kind();
    match cmd {
        "create" => {
            let store = SealedStore::create(
                &dir,
                StoreOptions {
                    mode: Some(Mode::Aes256Gcm),
                    source: Some(source),
                },
            )?;
            let mut w = store.create_segment(4096)?;
            w.append_block(b"probe block")?;
            w.commit()?;
        }
        "open" => {
            let store = SealedStore::open(&dir, Some(source))?;
            let id = store.segment_ids()[0];
            let block = store.open_segment_reader(id)?.read_block(0)?;
            anyhow::ensure!(&block[..] == b"probe block", "wrong block");
        }
        "delete" => {
            let mut source = source;
            source.delete()?;
        }
        _ => anyhow::bail!("unknown command"),
    }
    println!("{cmd} ok provider={kind}");
    Ok(())
}
