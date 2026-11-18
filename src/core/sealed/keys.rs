// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

#[cfg(feature = "security")]
use anyhow::anyhow;
use anyhow::{bail, ensure, Result};
use std::path::{Path, PathBuf};

#[cfg(feature = "security")]
pub(crate) type Key = zeroize::Zeroizing<[u8; 32]>;
#[cfg(not(feature = "security"))]
pub(crate) type Key = [u8; 32];

pub(crate) fn new_key(bytes: [u8; 32]) -> Key {
    #[cfg(feature = "security")]
    {
        zeroize::Zeroizing::new(bytes)
    }
    #[cfg(not(feature = "security"))]
    bytes
}

pub(crate) fn random_key() -> Result<Key> {
    let mut key = new_key([0; 32]);
    super::random_bytes(&mut key[..])?;
    Ok(key)
}

pub(crate) fn nonce_for(counter: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[4..].copy_from_slice(&counter.to_be_bytes());
    nonce
}

/// AES-256-GCM under one key, or the null cipher of plaintext mode. The detached tag is the
/// last 16 bytes of the combined output.
pub(crate) enum Cipher {
    Null,
    #[cfg(feature = "security")]
    Aes(Box<aes_gcm::Aes256Gcm>),
}

impl Cipher {
    #[inline(never)]
    pub(crate) fn aes(key: &[u8; 32]) -> Result<Self> {
        #[cfg(feature = "security")]
        {
            use aes_gcm::aead::KeyInit;
            let cipher = Box::new(
                aes_gcm::Aes256Gcm::new_from_slice(key)
                    .map_err(|_| anyhow!("AES-256-GCM init failed"))?,
            );
            crate::core::scrub_stack();
            Ok(Self::Aes(cipher))
        }
        #[cfg(not(feature = "security"))]
        {
            let _ = key;
            bail!("encryption requires the `security` feature")
        }
    }

    /// Returns `ciphertext || tag`.
    pub(crate) fn seal_with_nonce(
        &self,
        #[cfg_attr(not(feature = "security"), allow(unused_variables))] nonce: [u8; 12],
        aad: &[u8],
        plaintext: &[u8],
    ) -> Result<Vec<u8>> {
        match self {
            Self::Null => {
                let mut out = Vec::with_capacity(plaintext.len() + 16);
                out.extend_from_slice(plaintext);
                out.extend_from_slice(&null_tag(aad, plaintext));
                Ok(out)
            }
            #[cfg(feature = "security")]
            Self::Aes(cipher) => {
                use aes_gcm::aead::{Aead, Payload};
                let nonce = aes_gcm::Nonce::try_from(nonce.as_slice())
                    .map_err(|_| anyhow!("nonce construction failed"))?;
                cipher
                    .encrypt(
                        &nonce,
                        Payload {
                            msg: plaintext,
                            aad,
                        },
                    )
                    .map_err(|_| anyhow!("encryption failed"))
            }
        }
    }

    /// Opens `ciphertext || tag`.
    pub(crate) fn open_with_nonce(
        &self,
        #[cfg_attr(not(feature = "security"), allow(unused_variables))] nonce: [u8; 12],
        aad: &[u8],
        sealed: &[u8],
    ) -> Result<Vec<u8>> {
        ensure!(sealed.len() >= 16, "sealed data shorter than a tag");
        match self {
            Self::Null => {
                let (body, tag) = sealed.split_at(sealed.len() - 16);
                ensure!(tag == null_tag(aad, body), "integrity check failed");
                Ok(body.to_vec())
            }
            #[cfg(feature = "security")]
            Self::Aes(cipher) => {
                use aes_gcm::aead::{Aead, Payload};
                let nonce = aes_gcm::Nonce::try_from(nonce.as_slice())
                    .map_err(|_| anyhow!("nonce construction failed"))?;
                cipher
                    .decrypt(&nonce, Payload { msg: sealed, aad })
                    .map_err(|_| anyhow!("authentication failed"))
            }
        }
    }

    pub(crate) fn seal(
        &self,
        counter: u64,
        aad: &[u8],
        plaintext: &[u8],
    ) -> Result<(Vec<u8>, [u8; 16])> {
        let mut out = self.seal_with_nonce(nonce_for(counter), aad, plaintext)?;
        let mut tag = [0u8; 16];
        tag.copy_from_slice(&out[out.len() - 16..]);
        out.truncate(out.len() - 16);
        Ok((out, tag))
    }

    pub(crate) fn open(
        &self,
        counter: u64,
        aad: &[u8],
        ciphertext: &[u8],
        tag: &[u8; 16],
    ) -> Result<Vec<u8>> {
        let mut sealed = Vec::with_capacity(ciphertext.len() + 16);
        sealed.extend_from_slice(ciphertext);
        sealed.extend_from_slice(tag);
        self.open_with_nonce(nonce_for(counter), aad, &sealed)
    }
}

fn null_tag(aad: &[u8], body: &[u8]) -> [u8; 16] {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(aad);
    hasher.update(body);
    let mut tag = [0u8; 16];
    tag[..4].copy_from_slice(&hasher.finalize().to_le_bytes());
    tag
}

/// Where the 256-bit master key lives. There is no password derivation on the open path.
pub enum KeySource {
    /// Caller-supplied raw key; the caller persists any rotated key itself.
    Raw { current: Key, staged: Option<Key> },
    /// A key file (mode 0600 on Unix, exactly 32 bytes). Rotation stages `<path>.next`.
    File { path: PathBuf, dpapi: bool },
    /// The Linux Secret Service default collection (needs a session bus).
    #[cfg(all(feature = "keychain", target_os = "linux"))]
    SecretService { account: String },
    /// The macOS login keychain (generic password); rotation stages `<account>.next`.
    #[cfg(all(feature = "keychain", target_os = "macos"))]
    Keychain { service: String, account: String },
}

#[cfg(all(feature = "keychain", target_os = "macos"))]
mod keychain {
    use super::{new_key, Key};
    use anyhow::{anyhow, ensure, Result};
    use security_framework::passwords as pw;

    const NOT_FOUND: i32 = -25300; // errSecItemNotFound

    pub(super) fn get(service: &str, account: &str) -> Result<Option<Key>> {
        match pw::get_generic_password(service, account) {
            Ok(bytes) => {
                let bytes = new_zeroizing(bytes);
                ensure!(
                    bytes.len() == 32,
                    "keychain master key must be exactly 32 bytes"
                );
                let mut key = new_key([0; 32]);
                key.copy_from_slice(&bytes);
                Ok(Some(key))
            }
            Err(e) if e.code() == NOT_FOUND => Ok(None),
            Err(e) => Err(anyhow!("keychain read failed (status {})", e.code())),
        }
    }

    pub(super) fn set(service: &str, account: &str, key: &[u8; 32]) -> Result<()> {
        pw::set_generic_password(service, account, key)
            .map_err(|e| anyhow!("keychain write failed (status {})", e.code()))
    }

    pub(super) fn delete(service: &str, account: &str) -> Result<()> {
        match pw::delete_generic_password(service, account) {
            Err(e) if e.code() != NOT_FOUND => {
                Err(anyhow!("keychain delete failed (status {})", e.code()))
            }
            _ => Ok(()),
        }
    }

    fn new_zeroizing(v: Vec<u8>) -> zeroize::Zeroizing<Vec<u8>> {
        zeroize::Zeroizing::new(v)
    }
}

impl KeySource {
    pub fn raw(key: [u8; 32]) -> Self {
        Self::Raw {
            current: new_key(key),
            staged: None,
        }
    }

    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::File {
            path: path.into(),
            dpapi: false,
        }
    }

    /// The default provider for a store directory: the key file `key_file`, or
    /// `<dir>/master.key` when none is given.
    pub fn auto(dir: &Path, key_file: Option<PathBuf>) -> Self {
        let plain = key_file.clone().unwrap_or_else(|| dir.join("master.key"));
        // An existing key file always wins, so a store keeps opening with the provider
        // that created it.
        if key_file.is_none() && plain.symlink_metadata().is_err() {
            #[cfg(all(feature = "keychain", target_os = "macos"))]
            {
                let canonical = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
                let (service, account) = (
                    "com.writerslogic.warm".to_string(),
                    format!("store:{}", canonical.display()),
                );
                if keychain::get(&service, &account).is_ok() {
                    return Self::Keychain { service, account };
                }
            }
            #[cfg(all(feature = "keychain", windows))]
            return Self::File {
                path: dir.join("master.key.dpapi"),
                dpapi: true,
            };
            #[cfg(all(feature = "keychain", target_os = "linux"))]
            if secsvc::available() {
                let canonical = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
                return Self::SecretService {
                    account: format!("store:{}", canonical.display()),
                };
            }
        }
        Self::file(plain)
    }

    /// Provider name, for diagnostics.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Raw { .. } => "raw",
            Self::File { dpapi: true, .. } => "dpapi-file",
            Self::File { .. } => "key-file",
            #[cfg(all(feature = "keychain", target_os = "macos"))]
            Self::Keychain { .. } => "macos-keychain",
            #[cfg(all(feature = "keychain", target_os = "linux"))]
            Self::SecretService { .. } => "secret-service",
        }
    }

    /// Destroys the stored master key (and any staged one): every store sealed under it
    /// becomes unreadable. A `Raw` key is the caller's to drop.
    pub fn delete(&mut self) -> Result<()> {
        self.discard_staged()?;
        match self {
            Self::Raw { .. } => bail!("a raw key is owned by the caller"),
            Self::File { path, dpapi } => {
                scrub_key_file(path, *dpapi)?;
                match std::fs::remove_file(&*path) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                    _ => Ok(()),
                }
            }
            #[cfg(all(feature = "keychain", target_os = "macos"))]
            Self::Keychain { service, account } => keychain::delete(service, account),
            #[cfg(all(feature = "keychain", target_os = "linux"))]
            Self::SecretService { account } => secsvc::delete(account),
        }
    }

    fn staged_path(path: &Path) -> PathBuf {
        let mut name = path.as_os_str().to_owned();
        name.push(".next");
        PathBuf::from(name)
    }

    pub(crate) fn load(&self) -> Result<Option<Key>> {
        match self {
            Self::Raw { current, .. } => Ok(Some(new_key(*bytes(current)))),
            Self::File { path, dpapi } => read_key_file(path, *dpapi),
            #[cfg(all(feature = "keychain", target_os = "macos"))]
            Self::Keychain { service, account } => keychain::get(service, account),
            #[cfg(all(feature = "keychain", target_os = "linux"))]
            Self::SecretService { account } => secsvc::get(account),
        }
    }

    pub(crate) fn load_staged(&self) -> Result<Option<Key>> {
        match self {
            Self::Raw { staged, .. } => Ok(staged.as_ref().map(|k| new_key(*bytes(k)))),
            Self::File { path, dpapi } => read_key_file(&Self::staged_path(path), *dpapi),
            #[cfg(all(feature = "keychain", target_os = "macos"))]
            Self::Keychain { service, account } => {
                keychain::get(service, &format!("{account}.next"))
            }
            #[cfg(all(feature = "keychain", target_os = "linux"))]
            Self::SecretService { account } => secsvc::get(&format!("{account}.next")),
        }
    }

    /// Publishes a first key; refuses to replace an existing one.
    pub(crate) fn store_new(&mut self, key: &Key) -> Result<()> {
        match self {
            Self::Raw { current, .. } => {
                *current = new_key(*bytes(key));
                Ok(())
            }
            Self::File { path, dpapi } => {
                ensure!(
                    read_key_file(path, *dpapi)?.is_none(),
                    "a master key file already exists"
                );
                if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                    std::fs::create_dir_all(parent)?;
                }
                write_key_file(path, bytes(key), *dpapi)
            }
            #[cfg(all(feature = "keychain", target_os = "macos"))]
            Self::Keychain { service, account } => {
                ensure!(
                    keychain::get(service, account)?.is_none(),
                    "a master key already exists"
                );
                keychain::set(service, account, bytes(key))
            }
            #[cfg(all(feature = "keychain", target_os = "linux"))]
            Self::SecretService { account } => {
                ensure!(
                    secsvc::get(account)?.is_none(),
                    "a master key already exists"
                );
                secsvc::set(account, bytes(key))
            }
        }
    }

    pub(crate) fn stage(&mut self, key: &Key) -> Result<()> {
        match self {
            Self::Raw { staged, .. } => {
                *staged = Some(new_key(*bytes(key)));
                Ok(())
            }
            Self::File { path, dpapi } => {
                write_key_file(&Self::staged_path(path), bytes(key), *dpapi)
            }
            #[cfg(all(feature = "keychain", target_os = "macos"))]
            Self::Keychain { service, account } => {
                keychain::set(service, &format!("{account}.next"), bytes(key))
            }
            #[cfg(all(feature = "keychain", target_os = "linux"))]
            Self::SecretService { account } => secsvc::set(&format!("{account}.next"), bytes(key)),
        }
    }

    /// Makes the staged key current and destroys the previous key.
    pub(crate) fn commit_staged(&mut self) -> Result<()> {
        match self {
            Self::Raw { current, staged } => {
                if let Some(next) = staged.take() {
                    *current = next;
                }
                Ok(())
            }
            Self::File { path, dpapi } => {
                let next = Self::staged_path(path);
                if read_key_file(&next, *dpapi)?.is_none() {
                    return Ok(());
                }
                scrub_key_file(path, *dpapi)?;
                super::failpoint("rotate.after_scrub")?;
                std::fs::rename(&next, &*path)?;
                crate::core::durable_file::sync_directory(
                    path.parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or_else(|| Path::new(".")),
                )
            }
            #[cfg(all(feature = "keychain", target_os = "macos"))]
            Self::Keychain { service, account } => {
                let next_account = format!("{account}.next");
                if let Some(next) = keychain::get(service, &next_account)? {
                    keychain::set(service, account, bytes(&next))?;
                    keychain::delete(service, &next_account)?;
                }
                Ok(())
            }
            #[cfg(all(feature = "keychain", target_os = "linux"))]
            Self::SecretService { account } => {
                let next_account = format!("{account}.next");
                if let Some(next) = secsvc::get(&next_account)? {
                    secsvc::set(account, bytes(&next))?;
                    secsvc::delete(&next_account)?;
                }
                Ok(())
            }
        }
    }

    pub(crate) fn discard_staged(&mut self) -> Result<()> {
        match self {
            Self::Raw { staged, .. } => {
                *staged = None;
                Ok(())
            }
            Self::File { path, dpapi } => {
                let next = Self::staged_path(path);
                if next.symlink_metadata().is_ok() {
                    scrub_key_file(&next, *dpapi)?;
                    std::fs::remove_file(&next)?;
                }
                Ok(())
            }
            #[cfg(all(feature = "keychain", target_os = "macos"))]
            Self::Keychain { service, account } => {
                keychain::delete(service, &format!("{account}.next"))
            }
            #[cfg(all(feature = "keychain", target_os = "linux"))]
            Self::SecretService { account } => secsvc::delete(&format!("{account}.next")),
        }
    }
}

fn bytes(key: &Key) -> &[u8; 32] {
    key
}

fn scrub_key_file(path: &Path, _dpapi: bool) -> Result<()> {
    use std::io::{Seek, SeekFrom, Write};
    let mut file = match crate::core::durable_file::open_regular(path, true) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let len = file.metadata()?.len().min(MAX_KEY_FILE);
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&vec![0u8; len as usize])?;
    file.sync_all()?;
    Ok(())
}

/// Upper bound on a key file: 32 raw bytes, or a DPAPI blob.
const MAX_KEY_FILE: u64 = 4096;

fn write_key_file(path: &Path, key: &[u8; 32], dpapi: bool) -> Result<()> {
    if dpapi {
        #[cfg(all(feature = "keychain", windows))]
        return super::write_atomic(
            path,
            &dpapi::protect(key)?,
            true,
            ["key.before_rename", "key.after_rename"],
        );
        #[cfg(not(all(feature = "keychain", windows)))]
        bail!("DPAPI is only available on Windows");
    }
    super::write_atomic(path, key, true, ["key.before_rename", "key.after_rename"])
}

fn read_key_file(path: &Path, dpapi: bool) -> Result<Option<Key>> {
    use std::io::Read;
    let mut file = match crate::core::durable_file::open_regular(path, false) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let metadata = file.metadata()?;
    if dpapi {
        ensure!(
            (1..=MAX_KEY_FILE).contains(&metadata.len()),
            "protected key file has an invalid length"
        );
        let mut blob = vec![0u8; metadata.len() as usize];
        file.read_exact(&mut blob)?;
        #[cfg(all(feature = "keychain", windows))]
        return dpapi::unprotect(&blob).map(Some);
        #[cfg(not(all(feature = "keychain", windows)))]
        bail!("DPAPI is only available on Windows");
    }
    ensure!(
        metadata.len() == 32,
        "master key file must be exactly 32 bytes"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            bail!("master key file permissions must exclude group and other access");
        }
    }
    let mut key = new_key([0; 32]);
    file.read_exact(&mut key[..])?;
    Ok(Some(key))
}

/// Windows DPAPI, user scope: the master key sits in a file only the same Windows user can
/// unprotect. No UI is ever shown (`CRYPTPROTECT_UI_FORBIDDEN`).
#[cfg(all(feature = "keychain", windows))]
mod dpapi {
    use super::{new_key, Key};
    use anyhow::{anyhow, ensure, Result};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
    };

    const UI_FORBIDDEN: u32 = 0x1;

    fn take(blob: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        // SAFETY: on success `pbData` points to `cbData` bytes allocated by DPAPI; the copy
        // ends the borrow, the original is zeroed and then released with LocalFree.
        unsafe {
            let slice = std::slice::from_raw_parts_mut(blob.pbData, blob.cbData as usize);
            let out = slice.to_vec();
            slice.fill(0);
            LocalFree(blob.pbData.cast());
            out
        }
    }

    pub(super) fn protect(data: &[u8; 32]) -> Result<Vec<u8>> {
        let input = CRYPT_INTEGER_BLOB {
            cbData: 32,
            pbData: data.as_ptr().cast_mut(),
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: `input` describes 32 readable bytes that DPAPI does not modify; null
        // description, entropy, reserved and prompt pointers are permitted by the API.
        let ok = unsafe {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                UI_FORBIDDEN,
                &mut output,
            )
        };
        ensure!(ok != 0, "DPAPI protect failed");
        Ok(take(output))
    }

    pub(super) fn unprotect(blob: &[u8]) -> Result<Key> {
        let input = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(blob.len()).map_err(|_| anyhow!("protected blob too large"))?,
            pbData: blob.as_ptr().cast_mut(),
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: `input` describes `blob.len()` readable bytes; the description out
        // pointer is null (not requested), as are entropy, reserved and prompt.
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                UI_FORBIDDEN,
                &mut output,
            )
        };
        ensure!(ok != 0, "DPAPI unprotect failed");
        let plain = zeroizing(take(output));
        ensure!(
            plain.len() == 32,
            "protected master key has an invalid length"
        );
        let mut key = new_key([0; 32]);
        key.copy_from_slice(&plain);
        Ok(key)
    }

    fn zeroizing(v: Vec<u8>) -> zeroize::Zeroizing<Vec<u8>> {
        zeroize::Zeroizing::new(v)
    }
}

/// Linux Secret Service (libsecret's D-Bus API) through the default collection. Every call
/// runs on its own thread under a deadline, a locked collection is never unlocked (that
/// would prompt), and a missing session bus means "unavailable", so `auto` cannot hang on a
/// headless host.
#[cfg(all(feature = "keychain", target_os = "linux"))]
mod secsvc {
    use super::{new_key, Key};
    use anyhow::{anyhow, ensure, Result};
    use secret_service::blocking::{Collection, SecretService};
    use secret_service::EncryptionType;
    use std::collections::HashMap;
    use std::sync::mpsc;
    use std::time::Duration;

    const DEADLINE: Duration = Duration::from_secs(5);

    fn bounded<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(DEADLINE)
            .map_err(|_| anyhow!("secret service did not answer in time"))?
    }

    fn with_collection<T>(f: impl FnOnce(&Collection<'_>) -> Result<T>) -> Result<T> {
        let service = SecretService::connect(EncryptionType::Dh)
            .map_err(|e| anyhow!("secret service unavailable: {e}"))?;
        let collection = service
            .get_default_collection()
            .map_err(|e| anyhow!("secret service has no default collection: {e}"))?;
        ensure!(
            !collection.is_locked().map_err(|e| anyhow!("{e}"))?,
            "secret service collection is locked"
        );
        f(&collection)
    }

    fn attributes(account: &str) -> HashMap<&str, &str> {
        HashMap::from([("application", "warm"), ("account", account)])
    }

    pub(super) fn has_session_bus() -> bool {
        std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some_and(|v| !v.is_empty())
            || std::env::var_os("XDG_RUNTIME_DIR")
                .is_some_and(|d| std::path::Path::new(&d).join("bus").exists())
    }

    pub(super) fn available() -> bool {
        has_session_bus() && bounded(|| with_collection(|_| Ok(()))).is_ok()
    }

    pub(super) fn get(account: &str) -> Result<Option<Key>> {
        let account = account.to_owned();
        bounded(move || {
            with_collection(|collection| {
                let items = collection
                    .search_items(attributes(&account))
                    .map_err(|e| anyhow!("secret service search failed: {e}"))?;
                let Some(item) = items.first() else {
                    return Ok(None);
                };
                let secret = zeroizing(
                    item.get_secret()
                        .map_err(|e| anyhow!("secret service read failed: {e}"))?,
                );
                ensure!(
                    secret.len() == 32,
                    "stored master key must be exactly 32 bytes"
                );
                let mut key = new_key([0; 32]);
                key.copy_from_slice(&secret);
                Ok(Some(key))
            })
        })
    }

    pub(super) fn set(account: &str, key: &[u8; 32]) -> Result<()> {
        let (account, key) = (account.to_owned(), zeroizing(key.to_vec()));
        bounded(move || {
            with_collection(|collection| {
                collection
                    .create_item(
                        "WARM store master key",
                        attributes(&account),
                        &key,
                        true,
                        "application/octet-stream",
                    )
                    .map(|_| ())
                    .map_err(|e| anyhow!("secret service write failed: {e}"))
            })
        })
    }

    pub(super) fn delete(account: &str) -> Result<()> {
        let account = account.to_owned();
        bounded(move || {
            with_collection(|collection| {
                let items = collection
                    .search_items(attributes(&account))
                    .map_err(|e| anyhow!("secret service search failed: {e}"))?;
                for item in items {
                    item.delete()
                        .map_err(|e| anyhow!("secret service delete failed: {e}"))?;
                }
                Ok(())
            })
        })
    }

    fn zeroizing(v: Vec<u8>) -> zeroize::Zeroizing<Vec<u8>> {
        zeroize::Zeroizing::new(v)
    }
}
