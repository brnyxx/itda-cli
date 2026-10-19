//! Website accounts and their secrets.
//!
//! Account metadata (site, account id, label) lives in the local store; secret
//! values live in a [`SecretStore`], the OS keyring by default. Agents only
//! ever see account ids. The store also records which secret names each
//! account has, so removing an account removes its secrets without listing
//! the keyring.
mod secret;

pub use secret::{EnvStore, KeyringStore, MemoryStore, Secret, SecretKey, SecretStore};

use itda_store::{Migration, Store};
use itda_types::{
    error::{AppError, AppResult},
    util,
};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use std::sync::Arc;

static SCHEMA: Migration = Migration {
    component: "vault",
    steps: &["CREATE TABLE vault_account(site TEXT NOT NULL, account TEXT NOT NULL, label TEXT NOT NULL, created_ms INTEGER NOT NULL, PRIMARY KEY(site,account));
        CREATE TABLE vault_secret(site TEXT NOT NULL, account TEXT NOT NULL, name TEXT NOT NULL, updated_ms INTEGER NOT NULL, PRIMARY KEY(site,account,name), FOREIGN KEY(site,account) REFERENCES vault_account(site,account) ON DELETE CASCADE);"],
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Account {
    pub site: String,
    pub id: String,
    pub label: String,
    pub created_ms: i64,
    /// Names of the secrets this account has, never their values.
    pub secrets: Vec<String>,
}

pub struct Vault {
    store: Store,
    secrets: Arc<dyn SecretStore>,
}

impl Vault {
    pub fn open(store: Store, secrets: Arc<dyn SecretStore>) -> AppResult<Self> {
        store.migrate(&SCHEMA)?;
        Ok(Self { store, secrets })
    }

    /// Add an account, or relabel it if it exists.
    pub async fn add_account(&self, site: &str, id: &str, label: &str) -> AppResult<()> {
        SecretKey::new(site, id, "label")?;
        let (site, id, label) = (site.to_owned(), id.to_owned(), label.trim().to_owned());
        self.store
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO vault_account(site,account,label,created_ms) VALUES(?1,?2,?3,?4)
                     ON CONFLICT(site,account) DO UPDATE SET label=excluded.label",
                    params![site, id, label, util::unix_ms()],
                )?;
                Ok(())
            })
            .await
    }

    /// Accounts, for one site or all, with the names of their secrets.
    pub async fn accounts(&self, site: Option<&str>) -> AppResult<Vec<Account>> {
        let site = site.map(str::to_owned);
        self.store
            .call(move |conn| {
                let mut statement = conn.prepare(
                    "SELECT a.site, a.account, a.label, a.created_ms, COALESCE(GROUP_CONCAT(s.name, ','), '')
                     FROM vault_account a LEFT JOIN vault_secret s ON s.site=a.site AND s.account=a.account
                     WHERE ?1 IS NULL OR a.site=?1
                     GROUP BY a.site, a.account ORDER BY a.site, a.account",
                )?;
                let rows = statement.query_map([site], |r| {
                    let names: String = r.get(4)?;
                    let mut secrets: Vec<String> = names
                        .split(',')
                        .filter(|n| !n.is_empty())
                        .map(str::to_owned)
                        .collect();
                    secrets.sort();
                    Ok(Account {
                        site: r.get(0)?,
                        id: r.get(1)?,
                        label: r.get(2)?,
                        created_ms: r.get(3)?,
                        secrets,
                    })
                })?;
                Ok(rows.collect::<Result<_, _>>()?)
            })
            .await
    }

    /// Store a secret for an existing account.
    pub async fn set_secret(
        &self,
        site: &str,
        account: &str,
        name: &str,
        secret: Secret,
    ) -> AppResult<()> {
        let key = SecretKey::new(site, account, name)?;
        if !self.has_account(&key).await? {
            return Err(AppError::api("not_found", 404));
        }
        let secrets = self.secrets.clone();
        let stored = key.clone();
        blocking(move || secrets.set(&stored, &secret)).await?;
        self.store
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO vault_secret(site,account,name,updated_ms) VALUES(?1,?2,?3,?4)
                     ON CONFLICT(site,account,name) DO UPDATE SET updated_ms=excluded.updated_ms",
                    params![key.site, key.account, key.name, util::unix_ms()],
                )?;
                Ok(())
            })
            .await
    }

    /// Read a secret. A missing account or secret is `auth_required`, so the
    /// caller asks the user to reconnect instead of retrying.
    pub async fn secret(&self, site: &str, account: &str, name: &str) -> AppResult<Secret> {
        let key = SecretKey::new(site, account, name)?;
        if !self.has_account(&key).await? {
            return Err(AppError::api("auth_required", 401));
        }
        let secrets = self.secrets.clone();
        blocking(move || secrets.get(&key))
            .await?
            .ok_or_else(|| AppError::api("auth_required", 401))
    }

    /// Remove an account and every secret recorded for it.
    pub async fn remove_account(&self, site: &str, account: &str) -> AppResult<()> {
        SecretKey::new(site, account, "label")?;
        let (site, account) = (site.to_owned(), account.to_owned());
        let names: Vec<String> = {
            let (site, account) = (site.clone(), account.clone());
            self.store
                .call(move |conn| {
                    let mut statement =
                        conn.prepare("SELECT name FROM vault_secret WHERE site=?1 AND account=?2")?;
                    let rows = statement.query_map([site, account], |r| r.get(0))?;
                    Ok(rows.collect::<Result<_, _>>()?)
                })
                .await?
        };
        for name in names {
            let key = SecretKey::new(&site, &account, &name)?;
            let secrets = self.secrets.clone();
            blocking(move || secrets.delete(&key)).await?;
        }
        self.store
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM vault_account WHERE site=?1 AND account=?2",
                    [site, account],
                )?;
                Ok(())
            })
            .await
    }

    async fn has_account(&self, key: &SecretKey) -> AppResult<bool> {
        let (site, account) = (key.site.clone(), key.account.clone());
        self.store
            .call(move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT 1 FROM vault_account WHERE site=?1 AND account=?2",
                        [site, account],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some())
            })
            .await
    }
}

/// Keyring calls can block on the platform service, so they leave the async
/// runtime's worker threads.
async fn blocking<T, F>(job: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|_| AppError::api("internal", 500))?
}
