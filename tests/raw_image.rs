// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Writes the store half of the raw-device erasure check. Linux, and only when run by
//! `scripts/ci/raw-image-erase.sh`, which mounts a file-backed ext4 image at
//! `WARM_RAW_MOUNT`, runs this test, unmounts, and scans the image file for the markers
//! recorded in `WARM_RAW_MARKERS`.
//!
//! Three stores sit on the image. In `erased` the markers are forgotten with the default
//! `forget`. In `crashed` the process is ended at the third durability point of the purge's
//! scrubbing compaction and the store is reopened, which finishes the purge. In `control` the marker is
//! forgotten with `purge: false`; the scan must still find it, which shows the scan can see
//! a remnant at all. Every store also keeps a survivor whose marker must be found.
#![cfg(all(target_os = "linux", feature = "failpoints"))]

mod erase_common;

use erase_common::*;
use std::io::Read;
use std::path::PathBuf;
use writerslogic_warm::core::memory_api::*;

fn marker() -> String {
    let mut bytes = [0u8; 24];
    std::fs::File::open("/dev/urandom")
        .unwrap()
        .read_exact(&mut bytes)
        .unwrap();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("RAWMARK{hex}")
}

fn remember(memory: &Memory, name: &str, marker: &str) {
    memory
        .remember(RememberInput {
            id: Some(name.into()),
            text: format!("secret {marker} filler words for the {name} record"),
            ..RememberInput::default()
        })
        .unwrap();
}

fn forget(memory: &Memory, name: &str, purge: bool) {
    memory
        .forget(
            &ForgetScope::Ids {
                ids: vec![name.into()],
            },
            &ForgetOptions {
                purge,
                ..ForgetOptions::default()
            },
        )
        .unwrap();
}

/// Child: ends at the n-th durability point of the purge.
#[test]
fn raw_image_crash_child() {
    let (Ok(dir), Ok(m)) = (
        std::env::var("WARM_RAW_CHILD_DIR"),
        std::env::var("WARM_RAW_CHILD_MARKER"),
    ) else {
        return;
    };
    let memory = open(std::path::Path::new(&dir)).unwrap();
    remember(&memory, "keep", "RAWKEEPCRASHED");
    remember(&memory, "victim", &m);
    // The first half of a purge, committed: marker durable, delete durable, text still in the
    // journal. The process then ends inside the scrubbing compaction.
    memory.core().mark_purge_pending().unwrap();
    forget(&memory, "victim", false);
    writerslogic_warm::core::failpoint::arm(
        writerslogic_warm::core::failpoint::Fault::Exit,
        std::env::var("WARM_RAW_CHILD_POINT")
            .unwrap()
            .parse()
            .unwrap(),
        false,
    );
    memory.core().compact_scrubbing().unwrap();
}

#[test]
fn write_stores_for_the_raw_image_scan() {
    let (Ok(mount), Ok(out)) = (
        std::env::var("WARM_RAW_MOUNT"),
        std::env::var("WARM_RAW_MARKERS"),
    ) else {
        eprintln!("not run under scripts/ci/raw-image-erase.sh");
        return;
    };
    let mount = PathBuf::from(mount);
    let (erased, crashed, control) = (marker(), marker(), marker());

    let memory = open(&mount.join("erased")).unwrap();
    remember(&memory, "keep", "RAWKEEPERASED");
    remember(&memory, "victim", &erased);
    forget(&memory, "victim", true);
    drop(memory);

    // The purge is cut at its 3rd durability point; the reopen finishes it.
    let dir = mount.join("crashed");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "raw_image_crash_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("WARM_RAW_CHILD_DIR", &dir)
        .env("WARM_RAW_CHILD_MARKER", &crashed)
        .env("WARM_RAW_CHILD_POINT", "3")
        .output()
        .unwrap()
        .status;
    assert_eq!(
        status.code(),
        Some(writerslogic_warm::core::failpoint::EXIT_CODE)
    );
    drop(open(&dir).unwrap());

    let memory = open(&mount.join("control")).unwrap();
    remember(&memory, "keep", "RAWKEEPCONTROL");
    remember(&memory, "victim", &control);
    forget(&memory, "victim", false);
    // Closing a store compacts it, which would erase the control; leave it open.
    std::mem::forget(memory);

    std::fs::write(
        out,
        format!(
            "{{\"erased\":[\"{erased}\",\"{crashed}\"],\"present\":[\"{control}\",\"RAWKEEPERASED\",\"RAWKEEPCRASHED\",\"RAWKEEPCONTROL\"]}}"
        ),
    )
    .unwrap();
}
