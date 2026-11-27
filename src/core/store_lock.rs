// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use anyhow::{ensure, Context, Result};
use std::fs::{File, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub(crate) const LOCK_FILE: &str = ".hms.lock";

/// Process-scoped exclusive ownership of a writable WARM store.
///
/// Exactly one writer engine owns a store directory. The lock is an advisory whole-file
/// `flock` on Unix and a `LockFileEx` exclusive lock on Windows (both through
/// `std::fs::File::try_lock`), tied to the open file, so the OS drops it when the process
/// dies. Read-only engines (`WarmCore::open_read_only`) never touch this file: they read a
/// private snapshot of the committed files, so they neither wait for the writer nor make it
/// wait. On Windows the lock is mandatory, so another process cannot read the holder's pid
/// out of the file; the refusal then says the holder is unknown.
pub(crate) struct StoreLock {
    file: File,
    path: PathBuf,
}

impl StoreLock {
    pub(crate) fn acquire(store_path: &Path) -> Result<Self> {
        std::fs::create_dir_all(store_path)
            .with_context(|| format!("failed to create store {}", store_path.display()))?;
        let path = store_path.join(LOCK_FILE);
        let mut file = super::durable_file::open_regular_read_write_create(&path)
            .with_context(|| format!("failed to open store lock {}", path.display()))?;

        match super::durable_file::try_lock_exclusive(&file) {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(crate::core::error::StoreLockedError {
                    path: store_path.to_path_buf(),
                    holder_pid: read_holder_pid(&mut file),
                }
                .into());
            }
            Err(TryLockError::Error(error)) => {
                return Err(error)
                    .with_context(|| format!("failed to lock store {}", store_path.display()));
            }
        }

        ensure!(single_link(&file)?, "store lock must have exactly one link");

        // The pid is for a person inspecting the store; the lock is the `flock` above, which
        // nothing on disk records, so the write is not synced.
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        writeln!(file, "pid={}", std::process::id())?;

        Ok(Self { file, path })
    }
}

impl StoreLock {
    /// True if no engine holds the lock of the store directory `store_path` (or it has no
    /// lock file). Takes and releases the lock without writing anything.
    pub(crate) fn try_probe(store_path: &Path) -> Result<bool> {
        let path = store_path.join(LOCK_FILE);
        let file = match super::durable_file::open_regular_read_write(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
            Err(error) => return Err(error.into()),
        };
        match super::durable_file::try_lock_exclusive(&file) {
            Ok(()) => {
                super::durable_file::unlock(&file)?;
                Ok(true)
            }
            Err(TryLockError::WouldBlock) => Ok(false),
            Err(TryLockError::Error(error)) => Err(error.into()),
        }
    }
}

/// The pid the lock's owner wrote, if it can be read and parsed.
fn read_holder_pid(file: &mut File) -> Option<u32> {
    let mut text = String::new();
    file.seek(SeekFrom::Start(0)).ok()?;
    Read::by_ref(file).take(64).read_to_string(&mut text).ok()?;
    text.trim().strip_prefix("pid=")?.parse().ok()
}

#[cfg(unix)]
fn single_link(file: &File) -> std::io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    Ok(file.metadata()?.nlink() == 1)
}

#[cfg(windows)]
fn single_link(file: &File) -> std::io::Result<bool> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the handle and output buffer remain valid throughout the call.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(info.nNumberOfLinks == 1)
}

#[cfg(not(any(unix, windows)))]
fn single_link(_: &File) -> std::io::Result<bool> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "cannot verify store lock link count on this platform",
    ))
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        if let Err(error) = super::durable_file::unlock(&self.file) {
            tracing::warn!(path = %self.path.display(), %error, "failed to release WARM store lock");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_exclusive_and_released_on_drop() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let first = StoreLock::acquire(dir.path())?;
        let error = StoreLock::acquire(dir.path())
            .err()
            .expect("second lock must fail");
        assert!(error.to_string().contains("already open"));
        let locked = error
            .downcast_ref::<crate::core::error::StoreLockedError>()
            .expect("typed lock error");
        // Windows denies reads of a locked file, so only Unix can name the holder.
        match locked.holder_pid {
            Some(pid) => {
                assert_eq!(pid, std::process::id());
                assert!(error.to_string().contains(&format!("pid {pid}")));
            }
            None => assert!(error.to_string().contains("holder unknown")),
        }
        #[cfg(unix)]
        assert!(locked.holder_pid.is_some());
        drop(first);
        let reacquired = StoreLock::acquire(dir.path())?;
        drop(reacquired);
        Ok(())
    }

    /// The primitive behind every platform's lock (`flock` on Unix, `LockFileEx` on Windows):
    /// a second handle to the same file, even in this process, is refused until the first
    /// unlocks, and unlocking does not need the handle to close.
    #[test]
    fn the_lock_primitive_excludes_a_second_handle_until_unlocked() -> Result<()> {
        use super::super::durable_file::{
            open_regular_read_write_create, try_lock_exclusive, unlock,
        };
        let dir = tempfile::tempdir()?;
        let path = dir.path().join(LOCK_FILE);
        let first = open_regular_read_write_create(&path)?;
        let second = open_regular_read_write_create(&path)?;
        try_lock_exclusive(&first).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        assert!(matches!(
            try_lock_exclusive(&second),
            Err(TryLockError::WouldBlock)
        ));
        assert!(!StoreLock::try_probe(dir.path())?);
        unlock(&first)?;
        try_lock_exclusive(&second).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        unlock(&second)?;
        assert!(StoreLock::try_probe(dir.path())?);
        Ok(())
    }

    #[test]
    fn lock_refuses_hard_links_without_changing_the_target() -> Result<()> {
        let root = tempfile::tempdir()?;
        let store = root.path().join("store");
        std::fs::create_dir(&store)?;
        let target = root.path().join("external");
        std::fs::write(&target, b"external data")?;
        std::fs::hard_link(&target, store.join(LOCK_FILE))?;
        assert!(StoreLock::acquire(&store).is_err());
        assert_eq!(std::fs::read(&target)?, b"external data");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn lock_refuses_symlinks_without_changing_the_target() -> Result<()> {
        let root = tempfile::tempdir()?;
        let store = root.path().join("store");
        std::fs::create_dir(&store)?;
        let target = root.path().join("external");
        std::fs::write(&target, b"external data")?;
        std::os::unix::fs::symlink(&target, store.join(LOCK_FILE))?;
        assert!(StoreLock::acquire(&store).is_err());
        assert_eq!(std::fs::read(&target)?, b"external data");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn lock_refuses_a_fifo() -> Result<()> {
        use std::os::unix::ffi::OsStrExt;
        let root = tempfile::tempdir()?;
        let path = root.path().join(LOCK_FILE);
        let name = std::ffi::CString::new(path.as_os_str().as_bytes())?;
        // SAFETY: `name` is NUL-terminated and lives for the call.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(StoreLock::acquire(root.path()).is_err());
        Ok(())
    }
}
