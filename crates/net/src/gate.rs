//! Request pacing shared by every process using the same data directory.
//!
//! Each scope (`origin|account`, plus `origin|*` for the whole host) gets one
//! request at a time, a minimum interval and a ten-minute budget. A rate limit
//! sets a cooldown. A block, challenge, or a signed-in request answered with 401
//! pauses the scope until the user resumes it, so a refused site is never retried
//! in a loop.
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
use std::time::Duration;

pub const BLOCKED: &str = "blocked_by_site";
pub const AUTH_REQUIRED: &str = "auth_required";

static SCHEMA: Migration = Migration {
    component: "gate",
    steps: &["CREATE TABLE request_gate(scope TEXT PRIMARY KEY, lease TEXT NOT NULL DEFAULT '', lease_until INTEGER NOT NULL DEFAULT 0, next_at INTEGER NOT NULL DEFAULT 0, cooldown INTEGER NOT NULL DEFAULT 0, paused TEXT NOT NULL DEFAULT '');
        CREATE TABLE request_event(scope TEXT NOT NULL, at INTEGER NOT NULL);
        CREATE INDEX request_event_scope_time ON request_event(scope,at);"],
};

pub struct Gate {
    store: Store,
    interval_ms: i64,
    budget: i64,
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

impl Gate {
    /// The default pace: 1.2 seconds between requests, 60 per ten minutes.
    pub fn open(store: Store) -> AppResult<Self> {
        Self::with_limits(store, 1200, 60)
    }

    /// `interval_ms` between requests in one scope, at most `budget` requests per
    /// scope in any ten minutes.
    pub fn with_limits(store: Store, interval_ms: i64, budget: i64) -> AppResult<Self> {
        store.migrate(&SCHEMA)?;
        Ok(Self {
            store,
            interval_ms: interval_ms.max(0),
            budget: budget.max(1),
        })
    }

    /// Wait for this scope's turn and return a lease token. Fails at once when
    /// the scope is paused, cooling down or over budget.
    pub async fn acquire(&self, scope: &str) -> AppResult<String> {
        let start = util::unix_ms();
        loop {
            let token = util::id();
            let (scope_owned, token_owned) = (scope.to_owned(), token.clone());
            let (interval_ms, budget) = (self.interval_ms, self.budget);
            let wait = self
                .store
                .call(move |conn| {
                    try_acquire(conn, &scope_owned, &token_owned, interval_ms, budget)
                })
                .await?;
            if wait == 0 {
                return Ok(token);
            }
            if util::unix_ms() - start > 45_000 {
                return Err(AppError::api("timeout", 408));
            }
            tokio::time::sleep(Duration::from_millis(wait.min(500) as u64)).await;
        }
    }

    /// Release a lease. Only the holder of `token` can release or pause the scope.
    pub async fn release(&self, scope: &str, token: &str, outcome: Outcome) -> AppResult<()> {
        let (scope, token) = (scope.to_owned(), token.to_owned());
        self.store
            .call(move |conn| finish(conn, &scope, &token, outcome))
            .await
    }

    /// Release a lease from synchronous code (a guard's `Drop`) without waiting.
    pub fn release_detached(&self, scope: &str, token: &str, outcome: Outcome) {
        let (scope, token) = (scope.to_owned(), token.to_owned());
        self.store
            .submit(move |conn| finish(conn, &scope, &token, outcome));
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
    let now = util::unix_ms();
    if !paused.is_empty() {
        return Err(AppError::api(
            &paused,
            if paused == AUTH_REQUIRED { 401 } else { 403 },
        ));
    }
    if cooldown > now {
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
    let wait = (lease_until.max(next_at) - now).max(0);
    if wait == 0 {
        let jitter = if interval_ms >= 500 {
            ((now as u64 ^ token.as_bytes().first().copied().unwrap_or(0) as u64) % 180) as i64
        } else {
            0
        };
        tx.execute(
            "UPDATE request_gate SET lease=?2,lease_until=?3,next_at=?4 WHERE scope=?1",
            params![scope, token, now + 120_000, now + interval_ms + jitter],
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

fn finish(conn: &mut Connection, scope: &str, token: &str, outcome: Outcome) -> AppResult<()> {
    let (cooldown, paused) = match outcome {
        Outcome::Done => (0, ""),
        Outcome::RateLimited(retry_after) => (
            util::unix_ms()
                + retry_after
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
