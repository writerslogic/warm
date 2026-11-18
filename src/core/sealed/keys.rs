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
    File(PathBuf),
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
        Self::File(path.into())
    }

    /// The default provider for a store directory: the key file `key_file`, or
    /// `<dir>/master.key` when none is given.
    pub fn auto(dir: &Path, key_file: Option<PathBuf>) -> Self {
        #[cfg(all(feature = "keychain", target_os = "macos"))]
        if key_file.is_none() {
            let canonical = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
            let (service, account) = (
                "com.writerslogic.warm".to_string(),
                format!("store:{}", canonical.display()),
            );
            if keychain::get(&service, &account).is_ok() {
                return Self::Keychain { service, account };
            }
        }
        Self::File(key_file.unwrap_or_else(|| dir.join("master.key")))
    }

    fn staged_path(path: &Path) -> PathBuf {
        let mut name = path.as_os_str().to_owned();
        name.push(".next");
        PathBuf::from(name)
    }

    pub(crate) fn load(&self) -> Result<Option<Key>> {
        match self {
            Self::Raw { current, .. } => Ok(Some(new_key(*bytes(current)))),
            Self::File(path) => read_key_file(path),
            #[cfg(all(feature = "keychain", target_os = "macos"))]
            Self::Keychain { service, account } => keychain::get(service, account),
        }
    }

    pub(crate) fn load_staged(&self) -> Result<Option<Key>> {
        match self {
            Self::Raw { staged, .. } => Ok(staged.as_ref().map(|k| new_key(*bytes(k)))),
            Self::File(path) => read_key_file(&Self::staged_path(path)),
            #[cfg(all(feature = "keychain", target_os = "macos"))]
            Self::Keychain { service, account } => {
                keychain::get(service, &format!("{account}.next"))
            }
        }
    }

    /// Publishes a first key; refuses to replace an existing one.
    pub(crate) fn store_new(&mut self, key: &Key) -> Result<()> {
        match self {
            Self::Raw { current, .. } => {
                *current = new_key(*bytes(key));
                Ok(())
            }
            Self::File(path) => {
                ensure!(
                    read_key_file(path)?.is_none(),
                    "a master key file already exists"
                );
                if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                    std::fs::create_dir_all(parent)?;
                }
                super::write_atomic(
                    path,
                    bytes(key),
                    true,
                    ["key.before_rename", "key.after_rename"],
                )
            }
            #[cfg(all(feature = "keychain", target_os = "macos"))]
            Self::Keychain { service, account } => {
                ensure!(
                    keychain::get(service, account)?.is_none(),
                    "a master key already exists"
                );
                keychain::set(service, account, bytes(key))
            }
        }
    }

    pub(crate) fn stage(&mut self, key: &Key) -> Result<()> {
        match self {
            Self::Raw { staged, .. } => {
                *staged = Some(new_key(*bytes(key)));
                Ok(())
            }
            Self::File(path) => super::write_atomic(
                &Self::staged_path(path),
                bytes(key),
                true,
                ["key.before_rename", "key.after_rename"],
            ),
            #[cfg(all(feature = "keychain", target_os = "macos"))]
            Self::Keychain { service, account } => {
                keychain::set(service, &format!("{account}.next"), bytes(key))
            }
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
            Self::File(path) => {
                let next = Self::staged_path(path);
                if read_key_file(&next)?.is_none() {
                    return Ok(());
                }
                scrub_key_file(path)?;
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
        }
    }

    pub(crate) fn discard_staged(&mut self) -> Result<()> {
        match self {
            Self::Raw { staged, .. } => {
                *staged = None;
                Ok(())
            }
            Self::File(path) => {
                let next = Self::staged_path(path);
                if next.symlink_metadata().is_ok() {
                    scrub_key_file(&next)?;
                    std::fs::remove_file(&next)?;
                }
                Ok(())
            }
            #[cfg(all(feature = "keychain", target_os = "macos"))]
            Self::Keychain { service, account } => {
                keychain::delete(service, &format!("{account}.next"))
            }
        }
    }
}

fn bytes(key: &Key) -> &[u8; 32] {
    key
}

fn scrub_key_file(path: &Path) -> Result<()> {
    use std::io::{Seek, SeekFrom, Write};
    let mut file = match crate::core::durable_file::open_regular(path, true) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&[0u8; 32])?;
    file.sync_all()?;
    Ok(())
}

fn read_key_file(path: &Path) -> Result<Option<Key>> {
    use std::io::Read;
    let mut file = match crate::core::durable_file::open_regular(path, false) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let metadata = file.metadata()?;
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
