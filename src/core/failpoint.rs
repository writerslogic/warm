// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Fault injection at the store's durability points, for the crash and disk-fault tests.
//!
//! Every write-back, sync, rename, scrub and unlink of the commit and purge paths calls
//! [`hit`] with a stable name. Without the `failpoints` feature `hit` is an inlined `Ok(())`.
//! With it, the n-th hit (counted across all names, from 1) can be made to
//!
//! - `exit`: end the process at once with [`EXIT_CODE`] and no unwinding, which is what a
//!   killed process looks like to the files (the page cache survives, nothing is flushed);
//! - `enospc` or `eio`: return that I/O error, once (`:once`, the default) or from then on
//!   (`:sticky`, a disk that stays full or broken).
//!
//! A child process arms itself from `WARM_FAILPOINT=<kind>[:sticky]@<n>`;
//! `WARM_FAILPOINT_TRACE=<path>` appends each hit's name to a file. A test in the same process
//! uses [`arm`], [`disarm`] and [`hits`].

use std::io;

/// Exit status of a process ended by an `exit` failpoint.
pub const EXIT_CODE: i32 = 86;

/// What the armed hit does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// End the process without unwinding.
    Exit,
    /// Fail with "no space left on device".
    NoSpace,
    /// Fail with an I/O error.
    Io,
}

#[cfg(not(feature = "failpoints"))]
#[inline(always)]
pub(crate) fn hit(_name: &'static str) -> io::Result<()> {
    Ok(())
}

#[cfg(feature = "failpoints")]
pub(crate) use enabled::hit;
#[cfg(feature = "failpoints")]
pub use enabled::{arm, disarm, hits};

#[cfg(feature = "failpoints")]
mod enabled {
    use super::{io, Fault, EXIT_CODE};
    use std::io::Write;
    use std::sync::Mutex;

    struct State {
        hits: u64,
        at: Option<(Fault, u64, bool)>,
        trace: Option<std::path::PathBuf>,
        env_read: bool,
    }

    static STATE: Mutex<State> = Mutex::new(State {
        hits: 0,
        at: None,
        trace: None,
        env_read: false,
    });

    fn parse(spec: &str) -> Option<(Fault, u64, bool)> {
        let (kind, n) = spec.split_once('@')?;
        let (kind, sticky) = match kind.strip_suffix(":sticky") {
            Some(kind) => (kind, true),
            None => (kind.strip_suffix(":once").unwrap_or(kind), false),
        };
        let fault = match kind {
            "exit" => Fault::Exit,
            "enospc" => Fault::NoSpace,
            "eio" => Fault::Io,
            _ => return None,
        };
        Some((fault, n.parse().ok()?, sticky))
    }

    /// Arms the `n`-th hit from now (1 is the next) to inject `fault`; `sticky` keeps failing
    /// every hit after it. Resets the hit counter.
    pub fn arm(fault: Fault, n: u64, sticky: bool) {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.hits = 0;
        state.env_read = true;
        state.at = Some((fault, n, sticky));
    }

    /// Clears any armed fault and resets the hit counter.
    pub fn disarm() {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.hits = 0;
        state.env_read = true;
        state.at = None;
    }

    /// Hits since the last [`arm`] or [`disarm`].
    pub fn hits() -> u64 {
        STATE.lock().unwrap_or_else(|e| e.into_inner()).hits
    }

    #[cfg(windows)]
    fn error(fault: Fault) -> io::Error {
        // ERROR_DISK_FULL, ERROR_IO_DEVICE.
        io::Error::from_raw_os_error(if fault == Fault::NoSpace { 112 } else { 1117 })
    }

    #[cfg(not(windows))]
    fn error(fault: Fault) -> io::Error {
        // ENOSPC and EIO have the same numbers on Linux, macOS and the BSDs.
        io::Error::from_raw_os_error(if fault == Fault::NoSpace { 28 } else { 5 })
    }

    pub(crate) fn hit(name: &'static str) -> io::Result<()> {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if !state.env_read {
            state.env_read = true;
            state.at = std::env::var("WARM_FAILPOINT")
                .ok()
                .and_then(|spec| parse(&spec));
            state.trace = std::env::var_os("WARM_FAILPOINT_TRACE").map(Into::into);
        }
        state.hits += 1;
        if let Some(path) = &state.trace {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(file, "{name}");
            }
        }
        let Some((fault, at, sticky)) = state.at else {
            return Ok(());
        };
        if state.hits < at || (state.hits > at && !sticky) {
            return Ok(());
        }
        if fault == Fault::Exit {
            std::process::exit(EXIT_CODE);
        }
        drop(state);
        Err(error(fault))
    }
}
