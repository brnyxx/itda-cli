//! The local database. One SQLite file holds every component's tables; one
//! dedicated OS thread owns the connection, so async callers never block on
//! SQLite and no component opens its own connection.
//!
//! Each component declares its schema as numbered migration steps. Steps run
//! once, in order, and are recorded per component, so components evolve their
//! tables independently.
use itda_types::{
    error::{AppError, AppResult},
    util,
};
use rusqlite::Connection;
use std::{path::Path, sync::mpsc, thread, time::Duration};
use tokio::sync::oneshot;

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

/// A handle to the database thread. Cheap to clone; the thread stops when the
/// last handle is dropped.
#[derive(Clone)]
pub struct Store {
    jobs: mpsc::Sender<Job>,
}

/// Schema steps for one component. Step `n` (1-based) runs once, after steps
/// `1..n`. Never edit or reorder a released step; append a new one.
pub struct Migration {
    pub component: &'static str,
    pub steps: &'static [&'static str],
}

impl Store {
    /// Open (or create) the database at `path` in a private directory.
    pub fn open(path: &Path) -> AppResult<Self> {
        if let Some(parent) = path.parent() {
            util::private_dir(parent)?;
        }
        let path = path.to_owned();
        Self::start(move || Connection::open(path))
    }

    /// A database that lives only as long as the handle. For tests.
    pub fn in_memory() -> AppResult<Self> {
        Self::start(Connection::open_in_memory)
    }

    fn start(
        open: impl FnOnce() -> rusqlite::Result<Connection> + Send + 'static,
    ) -> AppResult<Self> {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel::<AppResult<()>>();
        thread::Builder::new()
            .name("itda-store".into())
            .spawn(move || {
                let mut conn = match open().and_then(|conn| {
                    conn.busy_timeout(Duration::from_secs(5))?;
                    conn.pragma_update(None, "journal_mode", "WAL")?;
                    conn.pragma_update(None, "foreign_keys", true)?;
                    conn.execute_batch(
                        "CREATE TABLE IF NOT EXISTS store_migration(component TEXT PRIMARY KEY, version INTEGER NOT NULL)",
                    )?;
                    Ok(conn)
                }) {
                    Ok(conn) => {
                        let _ = ready_tx.send(Ok(()));
                        conn
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.into()));
                        return;
                    }
                };
                for job in inbox {
                    job(&mut conn);
                }
            })
            .map_err(AppError::from)?;
        ready_rx.recv().map_err(|_| closed())??;
        Ok(Self { jobs })
    }

    /// Run `f` on the database thread and wait for it without blocking the
    /// async runtime.
    pub async fn call<T, F>(&self, f: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> AppResult<T> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.send(Box::new(move |conn| {
            let _ = tx.send(f(conn));
        }))?;
        rx.await.map_err(|_| closed())?
    }

    /// Run `f` on the database thread from synchronous code. Do not call this
    /// from inside an async task; use [`Store::call`] there.
    pub fn call_blocking<T, F>(&self, f: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> AppResult<T> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        self.send(Box::new(move |conn| {
            let _ = tx.send(f(conn));
        }))?;
        rx.recv().map_err(|_| closed())?
    }

    /// Queue `f` on the database thread without waiting for it. For cleanup
    /// that must happen from synchronous code such as `Drop`.
    pub fn submit<F>(&self, f: F)
    where
        F: FnOnce(&mut Connection) -> AppResult<()> + Send + 'static,
    {
        let _ = self.send(Box::new(move |conn| {
            let _ = f(conn);
        }));
    }

    /// Apply the steps of `migration` that have not run yet, in one
    /// transaction that takes the write lock before the version is read, so
    /// two processes starting at once cannot both see version 0 and both
    /// try to create the tables. Returns the component's schema version
    /// afterwards.
    pub fn migrate(&self, migration: &'static Migration) -> AppResult<usize> {
        self.call_blocking(move |conn| {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let mut version: usize = tx
                .query_row(
                    "SELECT version FROM store_migration WHERE component=?1",
                    [migration.component],
                    |row| row.get::<_, i64>(0),
                )
                .map(|v| v.max(0) as usize)
                .or_else(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => Ok(0),
                    other => Err(other),
                })?;
            if version > migration.steps.len() {
                return Err(AppError::new(
                    itda_types::Code::Runtime,
                    format!(
                        "A newer version wrote the local database ({} schema {version})",
                        migration.component
                    ),
                    "Update itda, or move the data directory aside.",
                ));
            }
            let before = version;
            while version < migration.steps.len() {
                tx.execute_batch(migration.steps[version])?;
                version += 1;
            }
            if version != before {
                tx.execute(
                    "INSERT INTO store_migration(component,version) VALUES(?1,?2)
                     ON CONFLICT(component) DO UPDATE SET version=excluded.version",
                    rusqlite::params![migration.component, version as i64],
                )?;
            }
            tx.commit()?;
            Ok(version)
        })
    }

    fn send(&self, job: Job) -> AppResult<()> {
        self.jobs.send(job).map_err(|_| closed())
    }
}

fn closed() -> AppError {
    AppError::new(
        itda_types::Code::Runtime,
        "The local database thread stopped",
        "Restart itda.",
    )
}
