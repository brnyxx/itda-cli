//! Request pacing shared by every process using the same data directory.
//!
//! Each scope (`origin|account`, plus `origin|*` for the whole host) gets one
//! request at a time, a minimum interval and a ten-minute budget. Callers in
//! one process line up in order; the lease itself lives in SQLite, so other
//! itda processes on the same data directory wait too. A rate limit sets a
//! cooldown: a short one (ten seconds or less) is waited out, a longer one
//! fails at once. A block, challenge, or a signed-in request answered with 401
//! pauses the scope until the user resumes it, so a refused site is never
//! retried in a loop.
//!
//! Derived from Hycli `src/policy.rs` (`acquire`, `release`, `resume`,
//! `resume_auth`), Apache-2.0. Changes: split out of the approval store and onto
//! the shared `itda-store` thread; pauses for blocks and expired sessions are
//! new; the transport strategy tables used to switch blocked requests to a
//! browser were removed.
use itda_store::{Migration, Store};
use itda_types::{
    error::{AppError, AppResult},
    util,
};
use rusqlite::{Connection, params};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::OwnedMutexGuard;

pub const BLOCKED: &str = "blocked_by_site";
pub const AUTH_REQUIRED: &str = "auth_required";

/// The pace every site gets unless its definition asks for more room.
pub const DEFAULT_INTERVAL: Duration = Duration::from_millis(1200);
/// A site definition cannot ask for more than this between requests.
pub const MAX_INTERVAL: Duration = Duration::from_secs(30);
/// A cooldown this long or shorter is waited out; a longer one fails at once.
pub const SHORT_COOLDOWN: Duration = Duration::from_secs(10);

/// Mark an error raised before any request left this machine, so a change
/// that fails this way is reported as not sent rather than as an unknown
/// outcome. See [`sent`].
pub fn not_sent(error: AppError) -> AppError {
    error.with("sent", false)
}

/// Whether the request this error ended may have reached the site. Errors
/// from waiting for a turn, resolving the name or connecting say `false`;
/// every other error leaves it open.
pub fn sent(error: &AppError) -> bool {
    error.extra.get("sent").and_then(serde_json::Value::as_bool) != Some(false)
}

static SCHEMA: Migration = Migration {
    component: "gate",
    steps: &["CREATE TABLE request_gate(scope TEXT PRIMARY KEY, lease TEXT NOT NULL DEFAULT '', lease_until INTEGER NOT NULL DEFAULT 0, next_at INTEGER NOT NULL DEFAULT 0, cooldown INTEGER NOT NULL DEFAULT 0, paused TEXT NOT NULL DEFAULT '');
        CREATE TABLE request_event(scope TEXT NOT NULL, at INTEGER NOT NULL);
        CREATE INDEX request_event_scope_time ON request_event(scope,at);"],
};

type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

pub struct Gate {
    store: Store,
    interval_ms: i64,
    budget: i64,
    clock: Clock,
    /// One queue per scope, so callers in this process take turns in order
    /// instead of polling the store against each other.
    slots: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

/// How a finished request should affect its scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// 429 or Retry-After: cool down for the given time (default one minute).
    RateLimited(Option<Duration>),
    /// Pause until the user resumes, with the public reason code.
    Pause(&'static str),
}

/// A turn on one scope. Dropping it without [`Lease::release`] releases it as
/// [`Outcome::Done`].
pub struct Lease<'a> {
    gate: &'a Gate,
    scope: String,
    token: String,
    released: bool,
    _slot: OwnedMutexGuard<()>,
}

impl std::fmt::Debug for Lease<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lease").field("scope", &self.scope).finish()
    }
}

impl Lease<'_> {
    /// Let the next caller through, recording how this request ended.
    pub async fn release(mut self, outcome: Outcome) -> AppResult<()> {
        self.gate.release(&self.scope, &self.token, outcome).await?;
        self.released = true;
        Ok(())
    }

    /// Keep the turn for another [`LEASE_MS`] while a long transfer runs.
    /// Fails with `request_lease_lost` when the turn is no longer this
    /// lease's: the transfer stops and is never retried.
    pub async fn renew(&self) -> AppResult<()> {
        let (scope, token, now) = (self.scope.clone(), self.token.clone(), self.gate.now());
        let kept = self
            .gate
            .store
            .call(move |conn| {
                Ok(conn.execute(
                    "UPDATE request_gate SET lease_until=?3 WHERE scope=?1 AND lease=?2",
                    params![scope, token, now + LEASE_MS],
                )?)
            })
            .await?;
        if kept == 1 {
            Ok(())
        } else {
            Err(AppError::api("request_lease_lost", 409))
        }
    }
}

/// How long a turn lasts once taken; a transfer longer than this renews it.
pub const LEASE_MS: i64 = 120_000;

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if !self.released {
            self.gate
                .release_detached(&self.scope, &self.token, Outcome::Done);
        }
    }
}

impl Gate {
    /// The default pace: 1.2 seconds between requests, 60 per ten minutes.
    pub fn open(store: Store) -> AppResult<Self> {
        Self::with_limits(store, DEFAULT_INTERVAL.as_millis() as i64, 60)
    }

    /// `interval_ms` between requests in one scope, at most `budget` requests per
    /// scope in any ten minutes.
    pub fn with_limits(store: Store, interval_ms: i64, budget: i64) -> AppResult<Self> {
        store.migrate(&SCHEMA)?;
        Ok(Self {
            store,
            interval_ms: interval_ms.max(0),
            budget: budget.max(1),
            clock: Arc::new(util::unix_ms),
            slots: Mutex::new(HashMap::new()),
        })
    }

    /// Read the time from `clock` (Unix milliseconds) instead of the system
    /// clock, so tests can run on a paused runtime.
    pub fn with_clock(mut self, clock: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    fn slot(&self, scope: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slots.len() >= 256 && !slots.contains_key(scope) {
            slots.retain(|_, slot| Arc::strong_count(slot) > 1);
        }
        slots.entry(scope.to_owned()).or_default().clone()
    }

    /// Wait for this scope's turn and return its lease. `interval` is the least
    /// time the site's definition wants between its requests; the gate's own
    /// interval is the floor. Fails when the scope is paused, over budget or
    /// cooling down for longer than [`SHORT_COOLDOWN`].
    pub async fn acquire(&self, scope: &str, interval: Option<Duration>) -> AppResult<Lease<'_>> {
        let start = self.now();
        let deadline = Duration::from_secs(45);
        let slot = tokio::time::timeout(deadline, self.slot(scope).lock_owned())
            .await
            .map_err(|_| not_sent(AppError::api("timeout", 408)))?;
        let interval_ms = interval
            .map(|wanted| wanted.min(MAX_INTERVAL).as_millis() as i64)
            .unwrap_or(0)
            .max(self.interval_ms);
        loop {
            let token = util::id();
            let (scope_owned, token_owned) = (scope.to_owned(), token.clone());
            let (budget, now) = (self.budget, self.now());
            let wait = self
                .store
                .call(move |conn| {
                    try_acquire(conn, &scope_owned, &token_owned, interval_ms, budget, now)
                })
                .await?;
            if wait == 0 {
                return Ok(Lease {
                    gate: self,
                    scope: scope.to_owned(),
                    token,
                    released: false,
                    _slot: slot,
                });
            }
            if self.now() - start > deadline.as_millis() as i64 {
                return Err(not_sent(AppError::api("timeout", 408)));
            }
            tokio::time::sleep(Duration::from_millis(wait.min(500) as u64)).await;
        }
    }

    /// Release a lease. Only the holder of `token` can release or pause the scope.
    async fn release(&self, scope: &str, token: &str, outcome: Outcome) -> AppResult<()> {
        let (scope, token, now) = (scope.to_owned(), token.to_owned(), self.now());
        self.store
            .call(move |conn| finish(conn, &scope, &token, outcome, now))
            .await
    }

    /// Release a lease from synchronous code (a guard's `Drop`) without waiting.
    fn release_detached(&self, scope: &str, token: &str, outcome: Outcome) {
        let (scope, token, now) = (scope.to_owned(), token.to_owned(), self.now());
        self.store
            .submit(move |conn| finish(conn, &scope, &token, outcome, now));
    }

    /// The reason a scope is paused, if it is.
    pub async fn paused(&self, scope: &str) -> AppResult<Option<String>> {
        let scope = scope.to_owned();
        self.store
            .call(move |conn| {
                let mut statement =
                    conn.prepare("SELECT paused FROM request_gate WHERE scope=?1")?;
                let mut rows = statement.query([scope])?;
                Ok(match rows.next()? {
                    Some(row) => Some(row.get::<_, String>(0)?).filter(|p| !p.is_empty()),
                    None => None,
                })
            })
            .await
    }

    /// The user reconnected or checked the site: clear pauses and cooldowns.
    pub async fn resume(&self, scope: &str) -> AppResult<()> {
        let scope = scope.to_owned();
        self.store
            .call(move |conn| {
                conn.execute(
                    "UPDATE request_gate SET paused='',cooldown=0 WHERE scope=?1",
                    [scope],
                )?;
                Ok(())
            })
            .await
    }
}

/// One attempt to take the lease. Returns how long to wait, or 0 when taken.
fn try_acquire(
    conn: &mut Connection,
    scope: &str,
    token: &str,
    interval_ms: i64,
    budget: i64,
    now: i64,
) -> AppResult<i64> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT INTO request_gate(scope) VALUES(?1) ON CONFLICT(scope) DO NOTHING",
        [scope],
    )?;
    let (lease_until, next_at, cooldown, paused): (i64, i64, i64, String) = tx.query_row(
        "SELECT lease_until,next_at,cooldown,paused FROM request_gate WHERE scope=?1",
        [scope],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    if !paused.is_empty() {
        return Err(AppError::api(
            &paused,
            if paused == AUTH_REQUIRED { 401 } else { 403 },
        ));
    }
    if cooldown - now > SHORT_COOLDOWN.as_millis() as i64 {
        return Err(AppError::api("rate_limited", 429));
    }
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM request_event WHERE scope=?1 AND at>?2",
        params![scope, now - 600_000],
        |r| r.get(0),
    )?;
    if count >= budget {
        return Err(AppError::api("rate_limited", 429));
    }
    let wait = (lease_until.max(next_at).max(cooldown) - now).max(0);
    if wait == 0 {
        let jitter = if interval_ms >= 500 {
            ((now as u64 ^ token.as_bytes().first().copied().unwrap_or(0) as u64) % 180) as i64
        } else {
            0
        };
        tx.execute(
            "UPDATE request_gate SET lease=?2,lease_until=?3,next_at=?4 WHERE scope=?1",
            params![scope, token, now + LEASE_MS, now + interval_ms + jitter],
        )?;
        tx.execute(
            "INSERT INTO request_event(scope,at) VALUES(?1,?2)",
            params![scope, now],
        )?;
        tx.execute("DELETE FROM request_event WHERE at<?1", [now - 3_600_000])?;
    }
    tx.commit()?;
    Ok(wait)
}

fn finish(
    conn: &mut Connection,
    scope: &str,
    token: &str,
    outcome: Outcome,
    now: i64,
) -> AppResult<()> {
    let (cooldown, paused) = match outcome {
        Outcome::Done => (0, ""),
        Outcome::RateLimited(retry_after) => (
            now + retry_after
                .unwrap_or(Duration::from_secs(60))
                .as_millis()
                .min(i64::MAX as u128) as i64,
            "",
        ),
        Outcome::Pause(reason) => (0, reason),
    };
    conn.execute(
        "UPDATE request_gate SET lease='',lease_until=0,cooldown=MAX(cooldown,?3),paused=?4 WHERE scope=?1 AND lease=?2",
        params![scope, token, cooldown, paused],
    )?;
    Ok(())
}
