//! Long secrets sealed into one file under the data directory.
//!
//! The OS credential store is the right place for an API key, but a browser
//! session is a whole `Cookie` header, and Windows Credential Manager caps a
//! credential blob at `CRED_MAX_CREDENTIAL_BLOB_SIZE` (5*512 = 2560 bytes;
//! wincred.h `CREDENTIAL`). So a secret longer than [`SEAL_OVER`], and every
//! browser session, goes into `vault.sealed`: AES-256-GCM over a JSON map of
//! `site/account/name` to value, with a fresh nonce for every write and one
//! 32-byte master key per data directory held in the credential store the
//! short secrets use. Short secrets stay where they were.
//!
//! Moving is silent and goes both ways: a read looks in the credential store
//! first and then in the file, a write puts the value where its length says
//! and removes it from the other place, and a delete clears both.
use crate::secret::{Secret, SecretKey, SecretStore};
use aws_lc_rs::aead::{AES_256_GCM, Aad, NONCE_LEN, Nonce, RandomizedNonceKey};
use itda_types::{
    error::{AppError, AppResult, Code},
    util,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Secrets longer than this many bytes are sealed into the file. It sits
/// under the Windows limit of 2560 bytes with room for the UTF-16 encoding
/// the credential store uses.
pub const SEAL_OVER: usize = 1024;

/// The file under the data directory that holds the sealed secrets.
pub const SEALED_FILE: &str = "vault.sealed";

/// The secret name every browser session is kept under; always sealed.
const SESSION: &str = "session";

const KEY_BYTES: usize = 32;
const FILE_LIMIT: usize = 16 * 1024 * 1024;

/// A credential store for short secrets, with the long ones sealed into a
/// file by a master key that lives in the same store.
pub struct SealedStore {
    path: PathBuf,
    lock: PathBuf,
    key_name: SecretKey,
    short: Arc<dyn SecretStore>,
}

#[derive(Serialize, Deserialize)]
struct Sealed {
    version: u32,
    nonce: String,
    sealed: String,
}

impl SealedStore {
    /// Seal long secrets under `home`, keeping short ones and the master key
    /// in `short`. The key's name carries a hash of `home`, so two data
    /// directories never share a key.
    pub fn new(home: &Path, short: Arc<dyn SecretStore>) -> Self {
        let tag: String = util::hash(home.to_string_lossy().as_bytes())
            .chars()
            .take(16)
            .collect();
        Self {
            path: home.join(SEALED_FILE),
            lock: home.join(format!("{SEALED_FILE}.lock")),
            key_name: SecretKey {
                site: "itda".into(),
                account: "vault".into(),
                name: format!("master-key-{tag}"),
            },
            short,
        }
    }

    /// Where the sealed file is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn sealed(key: &SecretKey, secret: &Secret) -> bool {
        key.name == SESSION || secret.expose().len() > SEAL_OVER
    }

    fn master_key(&self, create: bool) -> AppResult<Option<RandomizedNonceKey>> {
        let bytes = match self.short.get(&self.key_name)? {
            Some(encoded) => decode_hex(encoded.expose()).ok_or_else(unreadable)?,
            None if create => {
                let mut bytes = vec![0u8; KEY_BYTES];
                aws_lc_rs::rand::fill(&mut bytes).map_err(|_| sealing_failed())?;
                self.short
                    .set(&self.key_name, &Secret::new(encode_hex(&bytes)))?;
                bytes
            }
            None => return Ok(None),
        };
        RandomizedNonceKey::new(&AES_256_GCM, &bytes)
            .map(Some)
            .map_err(|_| unreadable())
    }

    /// Every sealed secret; an empty map when there is no file yet.
    fn read_all(&self) -> AppResult<BTreeMap<String, String>> {
        if !self.path.is_file() {
            return Ok(BTreeMap::new());
        }
        let file: Sealed = serde_json::from_slice(&util::read_bounded(&self.path, FILE_LIMIT)?)
            .map_err(|_| unreadable())?;
        if file.version != 1 {
            return Err(unreadable());
        }
        let key = self.master_key(false)?.ok_or_else(unreadable)?;
        let nonce = decode_hex(&file.nonce)
            .filter(|nonce| nonce.len() == NONCE_LEN)
            .and_then(|nonce| Nonce::try_assume_unique_for_key(&nonce).ok())
            .ok_or_else(unreadable)?;
        let mut bytes = decode_hex(&file.sealed).ok_or_else(unreadable)?;
        let plain = key
            .open_in_place(nonce, Aad::from(SEALED_FILE.as_bytes()), &mut bytes)
            .map_err(|_| unreadable())?;
        serde_json::from_slice(plain).map_err(|_| unreadable())
    }

    fn write_all(&self, secrets: &BTreeMap<String, String>) -> AppResult<()> {
        if secrets.is_empty() {
            if self.path.is_file() {
                std::fs::remove_file(&self.path)?;
            }
            return Ok(());
        }
        let key = self.master_key(true)?.ok_or_else(sealing_failed)?;
        let mut bytes = serde_json::to_vec(secrets)?;
        let nonce = key
            .seal_in_place_append_tag(Aad::from(SEALED_FILE.as_bytes()), &mut bytes)
            .map_err(|_| sealing_failed())?;
        let file = Sealed {
            version: 1,
            nonce: encode_hex(nonce.as_ref()),
            sealed: encode_hex(&bytes),
        };
        util::atomic_file(&self.path, &serde_json::to_vec(&file)?)
    }

    /// Change the sealed map under the file lock: writers in other processes
    /// wait, and each one starts from what is on disk.
    fn update(&self, change: impl FnOnce(&mut BTreeMap<String, String>) -> bool) -> AppResult<()> {
        let _lock = util::file_lock(&self.lock, false)?;
        let mut secrets = self.read_all()?;
        if change(&mut secrets) {
            self.write_all(&secrets)?;
        }
        Ok(())
    }
}

impl SecretStore for SealedStore {
    fn get(&self, key: &SecretKey) -> AppResult<Option<Secret>> {
        if let Some(secret) = self.short.get(key)? {
            return Ok(Some(secret));
        }
        if !self.path.is_file() {
            return Ok(None);
        }
        Ok(self.read_all()?.get(&key.path()).cloned().map(Secret::new))
    }

    fn set(&self, key: &SecretKey, secret: &Secret) -> AppResult<()> {
        let seal = Self::sealed(key, secret)
            || match self.short.set(key, secret) {
                Ok(()) => false,
                Err(error) if error.public_code == "secret_too_long" => true,
                Err(error) => return Err(error),
            };
        let path = key.path();
        if seal {
            let value = secret.expose().to_owned();
            self.update(|secrets| {
                secrets.insert(path.clone(), value);
                true
            })?;
            self.short.delete(key)
        } else {
            self.update(|secrets| secrets.remove(&path).is_some())
        }
    }

    fn delete(&self, key: &SecretKey) -> AppResult<()> {
        self.short.delete(key)?;
        if !self.path.is_file() {
            return Ok(());
        }
        let path = key.path();
        self.update(|secrets| secrets.remove(&path).is_some())
    }
}

fn unreadable() -> AppError {
    AppError::new(
        Code::Runtime,
        "The sealed secrets could not be read",
        "The master key in the system keyring no longer matches vault.sealed. Sign the affected sites in again.",
    )
}

fn sealing_failed() -> AppError {
    AppError::new(
        Code::Runtime,
        "The secret could not be sealed",
        "Check the data directory and the system keyring, then try again.",
    )
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let bytes = [0u8, 1, 15, 16, 255];
        assert_eq!(encode_hex(&bytes), "00010f10ff");
        assert_eq!(decode_hex("00010f10ff").as_deref(), Some(&bytes[..]));
        assert_eq!(decode_hex("abc"), None);
        assert_eq!(decode_hex("zz"), None);
    }
}
