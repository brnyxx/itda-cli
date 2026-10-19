//! Request pacing shared by every process using the same data directory.
//!
//! Each scope (`origin|account`, plus `origin|*` for the whole host) gets one
//! request at a time, a minimum interval and a ten-minute budget. A rate limit
//! sets a cooldown. A block, challenge, or a signed-in request answered with 401
//! pauses the scope until the user resumes it, so a refused site is never retried
//! in a loop.
//!
//! Derived from Hycli `src/policy.rs` (`acquire`, `release`, `resume`,
//! `resume_auth`), Apache-2.0. Changes: split out of the approval store; pauses
//! for blocks and expired sessions are new; the transport strategy tables used to
//! switch blocked requests to a browser were removed.
use itda_types::{
    error::{AppError, AppResult},
    util,
};
use rusqlite::{Connection, params};
use std::{path::Path, sync::Mutex, time::Duration};

pub const BLOCKED: &str = "blocked_by_site";
pub const AUTH_REQUIRED: &str = "auth_required";

pub struct Gate {
    conn: Mutex<Connection>,
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
    pub fn open(path: &Path) -> AppResult<Self> {
        Self::with_limits(path, 1200, 60)
    }

    /// `interval_ms` between requests in one scope, at most `budget` requests per
    /// scope in any ten minutes.
    pub fn with_limits(path: &Path, interval_ms: i64, budget: i64) -> AppResult<Self> {
        if let Some(parent) = path.parent() {
            util::private_dir(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS request_gate(scope TEXT PRIMARY KEY, lease TEXT NOT NULL DEFAULT '', lease_until INTEGER NOT NULL DEFAULT 0, next_at INTEGER NOT NULL DEFAULT 0, cooldown INTEGER NOT NULL DEFAULT 0, paused TEXT NOT NULL DEFAULT '');
            CREATE TABLE IF NOT EXISTS request_event(scope TEXT NOT NULL, at INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS request_event_scope_time ON request_event(scope,at);")?;
        Ok(Self {
            conn: Mutex::new(conn),
            interval_ms: interval_ms.max(0),
            budget: budget.max(1),
        })
    }

    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| AppError::api("internal", 500))
    }

    /// Wait for this scope's turn and return a lease token. Fails at once when
    /// the scope is paused, cooling down or over budget.
    pub async fn acquire(&self, scope: &str) -> AppResult<String> {
        let start = util::unix_ms();
        loop {
            let token = util::id();
            let wait = {
                let mut conn = self.lock()?;
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                tx.execute(
                    "INSERT INTO request_gate(scope) VALUES(?1) ON CONFLICT(scope) DO NOTHING",
                    [scope],
                )?;
                let (lease_until, next_at, cooldown, paused): (i64, i64, i64, String) = tx
                    .query_row(
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
                if count >= self.budget {
                    return Err(AppError::api("rate_limited", 429));
                }
                let wait = (lease_until.max(next_at) - now).max(0);
                if wait == 0 {
                    let jitter = if self.interval_ms >= 500 {
                        ((now as u64 ^ token.as_bytes().first().copied().unwrap_or(0) as u64) % 180)
                            as i64
                    } else {
                        0
                    };
                    tx.execute(
                        "UPDATE request_gate SET lease=?2,lease_until=?3,next_at=?4 WHERE scope=?1",
                        params![scope, token, now + 120_000, now + self.interval_ms + jitter],
                    )?;
                    tx.execute(
                        "INSERT INTO request_event(scope,at) VALUES(?1,?2)",
                        params![scope, now],
                    )?;
                    tx.execute("DELETE FROM request_event WHERE at<?1", [now - 3_600_000])?;
                }
                tx.commit()?;
                wait
            };
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
    pub fn release(&self, scope: &str, token: &str, outcome: Outcome) -> AppResult<()> {
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
        self.lock()?.execute(
            "UPDATE request_gate SET lease='',lease_until=0,cooldown=MAX(cooldown,?3),paused=?4 WHERE scope=?1 AND lease=?2",
            params![scope, token, cooldown, paused],
        )?;
        Ok(())
    }

    /// The reason a scope is paused, if it is.
    pub fn paused(&self, scope: &str) -> AppResult<Option<String>> {
        let conn = self.lock()?;
        let mut statement = conn.prepare("SELECT paused FROM request_gate WHERE scope=?1")?;
        let mut rows = statement.query([scope])?;
        Ok(match rows.next()? {
            Some(row) => Some(row.get::<_, String>(0)?).filter(|p| !p.is_empty()),
            None => None,
        })
    }

    /// The user reconnected or checked the site: clear pauses and cooldowns.
    pub fn resume(&self, scope: &str) -> AppResult<()> {
        self.lock()?.execute(
            "UPDATE request_gate SET paused='',cooldown=0 WHERE scope=?1",
            [scope],
        )?;
        Ok(())
    }
}
