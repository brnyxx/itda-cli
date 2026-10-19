#![allow(clippy::unwrap_used)]
use itda_store::Store;
use itda_vault::{EnvStore, MemoryStore, Secret, SecretKey, SecretStore, Vault};
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
