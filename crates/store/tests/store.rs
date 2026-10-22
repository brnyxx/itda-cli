#![allow(clippy::unwrap_used)]
use itda_store::{Migration, Store};

static NOTES: Migration = Migration {
    component: "notes",
    steps: &[
        "CREATE TABLE note(id INTEGER PRIMARY KEY, body TEXT NOT NULL)",
        "ALTER TABLE note ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0",
    ],
};

static NOTES_V1: Migration = Migration {
    component: "notes",
    steps: &["CREATE TABLE note(id INTEGER PRIMARY KEY, body TEXT NOT NULL)"],
};

#[test]
fn migrations_run_once_and_in_order() {
    let store = Store::in_memory().unwrap();
    assert_eq!(store.migrate(&NOTES_V1).unwrap(), 1);
    assert_eq!(store.migrate(&NOTES).unwrap(), 2);
    assert_eq!(store.migrate(&NOTES).unwrap(), 2);
    let pinned: i64 = store
        .call_blocking(|conn| {
            conn.execute("INSERT INTO note(body,pinned) VALUES('a',1)", [])?;
            Ok(conn.query_row("SELECT pinned FROM note", [], |r| r.get(0))?)
        })
        .unwrap();
    assert_eq!(pinned, 1);
}

#[test]
fn a_newer_schema_is_refused() {
    let store = Store::in_memory().unwrap();
    store.migrate(&NOTES).unwrap();
    assert!(store.migrate(&NOTES_V1).is_err());
}

#[test]
fn a_failed_step_leaves_the_previous_version() {
    static BROKEN: Migration = Migration {
        component: "broken",
        steps: &["CREATE TABLE ok(id INTEGER)", "THIS IS NOT SQL"],
    };
    static FIRST: Migration = Migration {
        component: "broken",
        steps: &["CREATE TABLE ok(id INTEGER)"],
    };
    let store = Store::in_memory().unwrap();
    assert!(store.migrate(&BROKEN).is_err());
    assert_eq!(store.migrate(&FIRST).unwrap(), 1);
}

#[tokio::test]
async fn async_calls_share_one_connection_across_tasks() {
    let dir = std::env::temp_dir().join(format!("itda-store-{}", itda_types::util::id()));
    let store = Store::open(&dir.join("itda.db")).unwrap();
    store.migrate(&NOTES).unwrap();
    let mut tasks = Vec::new();
    for i in 0..20 {
        let store = store.clone();
        tasks.push(tokio::spawn(async move {
            store
                .call(move |conn| {
                    conn.execute("INSERT INTO note(body) VALUES(?1)", [i.to_string()])?;
                    Ok(())
                })
                .await
        }));
    }
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    let count: i64 = store
        .call(|conn| Ok(conn.query_row("SELECT COUNT(*) FROM note", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(count, 20);
    drop(store);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn the_database_survives_reopening() {
    let dir = std::env::temp_dir().join(format!("itda-store-{}", itda_types::util::id()));
    let path = dir.join("itda.db");
    {
        let store = Store::open(&path).unwrap();
        store.migrate(&NOTES).unwrap();
        store
            .call_blocking(|conn| Ok(conn.execute("INSERT INTO note(body) VALUES('kept')", [])?))
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.migrate(&NOTES).unwrap(), 2);
    let body: String = store
        .call_blocking(|conn| Ok(conn.query_row("SELECT body FROM note", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(body, "kept");
    drop(store);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn submitted_jobs_run_before_later_calls() {
    let store = Store::in_memory().unwrap();
    store.migrate(&NOTES).unwrap();
    store.submit(|conn| {
        conn.execute("INSERT INTO note(body) VALUES('queued')", [])?;
        Ok(())
    });
    let count: i64 = store
        .call_blocking(|conn| Ok(conn.query_row("SELECT COUNT(*) FROM note", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn two_processes_starting_at_once_both_migrate_without_an_error() {
    let dir = std::env::temp_dir().join(format!("itda-store-{}", itda_types::util::id()));
    let path = dir.join("itda.db");
    // Both handles are opened here, one after the other: the race under test
    // is the migration, and a thread that failed to open would never reach
    // the barrier and leave the other waiting forever.
    let stores: Vec<Store> = (0..2).map(|_| Store::open(&path).unwrap()).collect();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let tasks: Vec<_> = stores
        .into_iter()
        .map(|store| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.migrate(&NOTES)
            })
        })
        .collect();
    for task in tasks {
        assert_eq!(task.join().unwrap().unwrap(), 2);
    }
    let store = Store::open(&path).unwrap();
    let version: i64 = store
        .call_blocking(|conn| {
            Ok(conn.query_row(
                "SELECT version FROM store_migration WHERE component='notes'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(version, 2);
    drop(store);
    std::fs::remove_dir_all(dir).ok();
}

#[cfg(unix)]
#[test]
fn the_database_and_its_wal_files_are_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("itda-store-{}", itda_types::util::id()));
    let path = dir.join("itda.db");
    std::fs::create_dir_all(&dir).unwrap();
    // A database left behind by an older itda, created with the umask.
    std::fs::write(&path, b"").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let store = Store::open(&path).unwrap();
    store.migrate(&NOTES).unwrap();
    store
        .call_blocking(|conn| Ok(conn.execute("INSERT INTO note(body) VALUES('kept')", [])?))
        .unwrap();
    let mode = |name: &str| {
        let file = dir.join(name);
        file.exists()
            .then(|| std::fs::metadata(&file).unwrap().permissions().mode() & 0o777)
    };
    assert_eq!(mode("itda.db"), Some(0o600));
    assert_eq!(
        mode("itda.db-wal"),
        Some(0o600),
        "the write-ahead log exists while the store is open and is private"
    );
    assert_eq!(mode("itda.db-shm"), Some(0o600));
    drop(store);

    // A fresh database is private from the start.
    let fresh = dir.join("fresh.db");
    let store = Store::open(&fresh).unwrap();
    store.migrate(&NOTES).unwrap();
    assert_eq!(mode("fresh.db"), Some(0o600));
    assert_eq!(mode("fresh.db-wal"), Some(0o600));
    assert_eq!(mode("fresh.db-shm"), Some(0o600));
    drop(store);
    std::fs::remove_dir_all(dir).ok();
}
