#![allow(clippy::unwrap_used)]
use itda_store::Store;
use itda_vault::{
    EnvStore, MemoryStore, SEAL_OVER, SEALED_FILE, SealedStore, Secret, SecretKey, SecretStore,
    Vault,
};
use std::sync::Arc;

fn vault() -> Vault {
    Vault::open(
        Store::in_memory().unwrap(),
        Arc::new(MemoryStore::default()),
    )
    .unwrap()
}

#[tokio::test]
async fn secrets_belong_to_accounts_and_are_listed_by_name_only() {
    let vault = vault();
    vault.add_account("intranet", "me", "Me").await.unwrap();
    vault
        .set_secret("intranet", "me", "api-key", Secret::new("s3cr3t-value"))
        .await
        .unwrap();
    assert_eq!(
        vault
            .secret("intranet", "me", "api-key")
            .await
            .unwrap()
            .expose(),
        "s3cr3t-value"
    );

    let accounts = vault.accounts(Some("intranet")).await.unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].secrets, vec!["api-key".to_string()]);
    let listed = serde_json::to_string(&accounts).unwrap();
    assert!(!listed.contains("s3cr3t-value"));

    // Relabelling keeps the secrets.
    vault.add_account("intranet", "me", "Mina").await.unwrap();
    let accounts = vault.accounts(None).await.unwrap();
    assert_eq!(accounts[0].label, "Mina");
    assert_eq!(accounts[0].secrets.len(), 1);
}

#[tokio::test]
async fn missing_accounts_and_secrets_ask_to_reconnect() {
    let vault = vault();
    assert_eq!(
        vault
            .secret("intranet", "me", "api-key")
            .await
            .unwrap_err()
            .public_code,
        "auth_required"
    );
    assert_eq!(
        vault
            .set_secret("intranet", "me", "api-key", Secret::new("x"))
            .await
            .unwrap_err()
            .public_code,
        "not_found"
    );
    vault.add_account("intranet", "me", "").await.unwrap();
    assert_eq!(
        vault
            .secret("intranet", "me", "api-key")
            .await
            .unwrap_err()
            .public_code,
        "auth_required"
    );
}

#[tokio::test]
async fn removing_an_account_removes_its_secrets() {
    let secrets = Arc::new(MemoryStore::default());
    let vault = Vault::open(Store::in_memory().unwrap(), secrets.clone()).unwrap();
    vault.add_account("intranet", "me", "").await.unwrap();
    vault
        .set_secret("intranet", "me", "api-key", Secret::new("x"))
        .await
        .unwrap();
    vault.remove_account("intranet", "me").await.unwrap();

    let key = SecretKey::new("intranet", "me", "api-key").unwrap();
    assert!(secrets.get(&key).unwrap().is_none());
    assert!(vault.accounts(None).await.unwrap().is_empty());
}

#[tokio::test]
async fn invalid_names_are_refused() {
    let vault = vault();
    for (site, account) in [("Intranet", "me"), ("intranet", "a/b"), ("intranet", "")] {
        assert!(vault.add_account(site, account, "").await.is_err());
    }
}

#[test]
fn secrets_never_print() {
    let secret = Secret::new("s3cr3t-value");
    assert_eq!(format!("{secret:?}"), "Secret(***)");
    assert_ne!(secret.fingerprint(), "s3cr3t-value");
    assert_eq!(
        secret.fingerprint(),
        Secret::new("s3cr3t-value").fingerprint()
    );
}

#[test]
fn environment_secrets_are_read_only() {
    let key = SecretKey::new("intranet", "ci-bot", "api-key").unwrap();
    assert_eq!(key.env_var(), "ITDA_SECRET__INTRANET__CI_BOT__API_KEY");
    let env = EnvStore::from_vars([
        (key.env_var(), "from-env".to_string()),
        ("PATH".to_string(), "/bin".to_string()),
    ]);
    assert_eq!(env.get(&key).unwrap().unwrap().expose(), "from-env");
    assert!(env.set(&key, &Secret::new("x")).is_err());
    let other = SecretKey::new("intranet", "me", "api-key").unwrap();
    assert!(env.get(&other).unwrap().is_none());
}

#[tokio::test]
async fn environment_secrets_need_no_registered_account() {
    let key = SecretKey::new("intranet", "ci-bot", "api-key").unwrap();
    let env = EnvStore::from_vars([(key.env_var(), "from-env".to_string())]);
    let vault = Vault::open(Store::in_memory().unwrap(), Arc::new(env)).unwrap();
    let secret = vault.secret("intranet", "ci-bot", "api-key").await.unwrap();
    assert_eq!(secret.expose(), "from-env");
}

fn sealed_home() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("itda-vault-sealed-{}", itda_types::util::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn long_secrets_and_sessions_are_sealed_into_a_file_and_short_ones_are_not() {
    let home = sealed_home();
    let keys = Arc::new(MemoryStore::default());
    let store = SealedStore::new(&home, keys.clone());
    let key = SecretKey::new("site", "default", "api-key").unwrap();
    let session = SecretKey::new("site", "default", "session").unwrap();
    let long = Secret::new("c".repeat(SEAL_OVER + 1));
    let short = Secret::new("short-key-value");
    let cookies = Secret::new("sid=abc; other=def");

    store.set(&key, &short).unwrap();
    assert_eq!(keys.get(&key).unwrap(), Some(short.clone()));
    assert!(
        !home.join(SEALED_FILE).exists(),
        "a short secret needs no file"
    );

    store.set(&session, &cookies).unwrap();
    store.set(&key, &long).unwrap();
    assert_eq!(
        keys.get(&key).unwrap(),
        None,
        "moved out of the credential store"
    );
    assert_eq!(keys.get(&session).unwrap(), None);
    assert_eq!(store.get(&key).unwrap(), Some(long.clone()));
    assert_eq!(store.get(&session).unwrap(), Some(cookies.clone()));

    // The file holds no plaintext, only the master key is in the credential store.
    let file = std::fs::read_to_string(home.join(SEALED_FILE)).unwrap();
    assert!(!file.contains("sid=abc"));
    assert!(!file.contains("ccccc"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(home.join(SEALED_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // Another process over the same directory and keys reads the same secrets.
    let again = SealedStore::new(&home, keys.clone());
    assert_eq!(again.get(&session).unwrap(), Some(cookies));

    // A secret that shrinks goes back to the credential store, and out of the file.
    store.set(&key, &short).unwrap();
    assert_eq!(keys.get(&key).unwrap(), Some(short.clone()));
    assert_eq!(again.get(&key).unwrap(), Some(short));
    assert!(
        !std::fs::read_to_string(home.join(SEALED_FILE))
            .unwrap()
            .is_empty()
    );

    store.delete(&session).unwrap();
    assert_eq!(store.get(&session).unwrap(), None);
    assert!(!home.join(SEALED_FILE).exists(), "an empty file is removed");
    std::fs::remove_dir_all(home).ok();
}

#[test]
fn a_sealed_file_without_its_key_fails_plainly() {
    let home = sealed_home();
    let keys = Arc::new(MemoryStore::default());
    let store = SealedStore::new(&home, keys.clone());
    let session = SecretKey::new("site", "default", "session").unwrap();
    store.set(&session, &Secret::new("sid=1")).unwrap();

    // Another directory gets its own key and sees nothing of this one.
    let elsewhere = sealed_home();
    let other = SealedStore::new(&elsewhere, keys.clone());
    assert_eq!(other.get(&session).unwrap(), None);

    // The key is gone: the file cannot be read, and the error says so.
    let without = SealedStore::new(&home, Arc::new(MemoryStore::default()));
    let error = without.get(&session).unwrap_err();
    assert!(error.msg.contains("could not read"), "{}", error.msg);
    assert!(error.remedy.contains("Sign the affected sites in again"));
    std::fs::remove_dir_all(home).ok();
    std::fs::remove_dir_all(elsewhere).ok();
}

#[tokio::test]
async fn a_browser_session_in_the_vault_is_sealed() {
    let home = sealed_home();
    let keys = Arc::new(MemoryStore::default());
    let vault = Vault::open(
        Store::in_memory().unwrap(),
        Arc::new(SealedStore::new(&home, keys.clone())),
    )
    .unwrap();
    vault.add_account("site", "default", "").await.unwrap();
    vault
        .set_secret("site", "default", "session", Secret::new("sid=1"))
        .await
        .unwrap();
    assert!(home.join(SEALED_FILE).exists());
    assert_eq!(
        vault
            .secret("site", "default", "session")
            .await
            .unwrap()
            .expose(),
        "sid=1"
    );
    vault.remove_account("site", "default").await.unwrap();
    assert!(vault.secret("site", "default", "session").await.is_err());
    assert!(!home.join(SEALED_FILE).exists());
    std::fs::remove_dir_all(home).ok();
}
