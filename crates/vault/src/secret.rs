//! Secret values and the places they are kept.
//!
//! A secret never appears in `Debug` output, and its key names only the site,
//! account and secret name. `KeyringStore` is the default; `EnvStore` serves
//! headless runs and CI, and `MemoryStore` serves tests.
use itda_types::{
    error::{AppError, AppResult, Code},
    util,
};
use std::{collections::HashMap, fmt, sync::Mutex};

/// A secret value. `Debug` never prints it; read it with [`Secret::expose`].
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    /// A stable hash for binding approvals to the credential in use, without
    /// storing the credential.
    pub fn fingerprint(&self) -> String {
        util::hash(self.0.as_bytes())
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// Where a secret lives: `site/account/name`, each part a lowercase id.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SecretKey {
    pub site: String,
    pub account: String,
    pub name: String,
}

impl SecretKey {
    pub fn new(site: &str, account: &str, name: &str) -> AppResult<Self> {
        if ![site, account, name]
            .iter()
            .all(|part| util::valid_id(part))
        {
            return Err(AppError::api("bad_request", 400));
        }
        Ok(Self {
            site: site.into(),
            account: account.into(),
            name: name.into(),
        })
    }

    pub(crate) fn path(&self) -> String {
        format!("{}/{}/{}", self.site, self.account, self.name)
    }

    /// `ITDA_SECRET__<SITE>__<ACCOUNT>__<NAME>`, with `-` written as `_`.
    pub fn env_var(&self) -> String {
        let part = |s: &str| s.to_ascii_uppercase().replace('-', "_");
        format!(
            "ITDA_SECRET__{}__{}__{}",
            part(&self.site),
            part(&self.account),
            part(&self.name)
        )
    }
}

/// A place secrets are kept. Calls may block; the vault runs them off the
/// async runtime.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &SecretKey) -> AppResult<Option<Secret>>;
    fn set(&self, key: &SecretKey, secret: &Secret) -> AppResult<()>;
    fn delete(&self, key: &SecretKey) -> AppResult<()>;
}

/// The operating system's credential store: Keychain on macOS, Credential
/// Manager on Windows, Secret Service elsewhere.
pub struct KeyringStore {
    service: String,
}

impl KeyringStore {
    pub fn new() -> Self {
        Self {
            service: "itda".into(),
        }
    }

    /// Whether the platform credential store could be opened.
    pub fn available() -> AppResult<()> {
        // Any failure to open the store means it is unavailable, whatever the
        // platform reported.
        keyring::Entry::store_status()
            .as_ref()
            .map_err(|_| keyring_error(&keyring::Error::NoDefaultStore))
            .copied()
    }

    fn entry(&self, key: &SecretKey) -> AppResult<keyring::Entry> {
        keyring::Entry::new(&self.service, &key.path()).map_err(|e| keyring_error(&e))
    }
}

impl Default for KeyringStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, key: &SecretKey) -> AppResult<Option<Secret>> {
        match self.entry(key)?.get_password() {
            Ok(value) => Ok(Some(Secret(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keyring_error(&e)),
        }
    }

    fn set(&self, key: &SecretKey, secret: &Secret) -> AppResult<()> {
        self.entry(key)?
            .set_password(&secret.0)
            .map_err(|e| keyring_error(&e))
    }

    fn delete(&self, key: &SecretKey) -> AppResult<()> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keyring_error(&e)),
        }
    }
}

/// Keyring errors are reported by kind only: some variants carry the bytes
/// that failed to decode, which may be part of a secret.
fn keyring_error(error: &keyring::Error) -> AppError {
    let msg = match error {
        keyring::Error::NoDefaultStore | keyring::Error::NoStorageAccess(_) => {
            "The system keyring is not available"
        }
        keyring::Error::BadEncoding(_) | keyring::Error::BadDataFormat(..) => {
            "A stored secret could not be read"
        }
        keyring::Error::TooLong(..) => "The secret is too long for the system keyring",
        _ => "The system keyring refused the operation",
    };
    let mut app_error = AppError::new(
        Code::Runtime,
        msg,
        "Unlock the system keyring, or set the secret in an ITDA_SECRET__ environment variable for headless runs.",
    );
    // `SealedStore` catches this one and seals the secret into its file.
    if matches!(error, keyring::Error::TooLong(..)) {
        app_error.public_code = "secret_too_long".into();
    }
    app_error
}

/// Read-only secrets from environment variables named by [`SecretKey::env_var`].
pub struct EnvStore {
    vars: HashMap<String, String>,
}

impl EnvStore {
    /// Snapshot the `ITDA_SECRET__` variables of this process.
    pub fn from_env() -> Self {
        Self::from_vars(std::env::vars())
    }

    pub fn from_vars(vars: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            vars: vars
                .into_iter()
                .filter(|(name, _)| name.starts_with("ITDA_SECRET__"))
                .collect(),
        }
    }
}

impl SecretStore for EnvStore {
    fn get(&self, key: &SecretKey) -> AppResult<Option<Secret>> {
        Ok(self.vars.get(&key.env_var()).cloned().map(Secret))
    }

    fn set(&self, _: &SecretKey, _: &Secret) -> AppResult<()> {
        Err(AppError::new(
            Code::Usage,
            "Environment secrets are read-only",
            "Set the ITDA_SECRET__ variable before starting, or use the system keyring.",
        ))
    }

    fn delete(&self, key: &SecretKey) -> AppResult<()> {
        self.set(key, &Secret::new(""))
    }
}

/// Secrets held in memory for tests and one-off runs.
#[derive(Default)]
pub struct MemoryStore {
    values: Mutex<HashMap<SecretKey, Secret>>,
}

impl MemoryStore {
    fn values(&self) -> AppResult<std::sync::MutexGuard<'_, HashMap<SecretKey, Secret>>> {
        self.values
            .lock()
            .map_err(|_| AppError::api("internal", 500))
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, key: &SecretKey) -> AppResult<Option<Secret>> {
        Ok(self.values()?.get(key).cloned())
    }

    fn set(&self, key: &SecretKey, secret: &Secret) -> AppResult<()> {
        self.values()?.insert(key.clone(), secret.clone());
        Ok(())
    }

    fn delete(&self, key: &SecretKey) -> AppResult<()> {
        self.values()?.remove(key);
        Ok(())
    }
}
