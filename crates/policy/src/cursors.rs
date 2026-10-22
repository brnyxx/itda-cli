//! Page markers kept for the agent. A read that turns pages gets the site's
//! next marker (a cursor, a page number, an offset) from the answer; the
//! marker is stored here and the agent receives an opaque handle bound to
//! the exact call it continues: site, action, account, the other inputs,
//! the definition and the credential. The handle lives fifteen minutes,
//! works for that call only, and the raw marker never reaches the agent.
use itda_store::{Migration, Store};
use itda_types::{
    error::{AppError, AppResult},
    util,
};
use rusqlite::{OptionalExtension, params};

const HANDLE_TTL_MS: i64 = 15 * 60 * 1000;
/// Handles kept at most; the oldest go first.
const HANDLE_CAP: usize = 256;
const PREFIX: &str = "ic_";

static SCHEMA: Migration = Migration {
    component: "cursors",
    steps: &[
        "CREATE TABLE cursors(handle TEXT PRIMARY KEY, binding TEXT NOT NULL, value TEXT NOT NULL, expires_ms INTEGER NOT NULL);",
    ],
};

pub struct Cursors {
    store: Store,
}

impl Cursors {
    pub fn open(store: Store) -> AppResult<Self> {
        store.migrate(&SCHEMA)?;
        Ok(Self { store })
    }

    /// Keep `value` for the call `binding` describes and return the handle
    /// the agent gets instead.
    pub async fn issue(&self, binding: &str, value: &str) -> AppResult<String> {
        let handle = format!("{PREFIX}{}", util::id());
        let row = (handle.clone(), binding.to_owned(), value.to_owned());
        self.store
            .call(move |conn| {
                let now = util::unix_ms();
                conn.execute("DELETE FROM cursors WHERE expires_ms<=?1", [now])?;
                conn.execute(
                    "INSERT INTO cursors(handle,binding,value,expires_ms) VALUES(?1,?2,?3,?4)",
                    params![row.0, row.1, row.2, now + HANDLE_TTL_MS],
                )?;
                conn.execute(
                    "DELETE FROM cursors WHERE rowid IN (SELECT rowid FROM cursors ORDER BY rowid DESC LIMIT -1 OFFSET ?1)",
                    [HANDLE_CAP as i64],
                )?;
                Ok(())
            })
            .await?;
        Ok(handle)
    }

    /// The marker behind `handle`, for the call `binding` describes. A
    /// handle that is not itda's, has expired or was issued for another
    /// call is refused with a plain message.
    pub async fn resolve(&self, handle: &str, binding: &str) -> AppResult<String> {
        if !well_formed(handle) {
            return Err(invalid("next is not a handle itda gave out"));
        }
        let (handle, binding) = (handle.to_owned(), binding.to_owned());
        let row: Option<(String, i64)> = self
            .store
            .call(move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT value, expires_ms FROM cursors WHERE handle=?1 AND binding=?2",
                        params![handle, binding],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?)
            })
            .await?;
        match row {
            Some((value, expires)) if expires > util::unix_ms() => Ok(value),
            Some(_) => Err(AppError::api("pagination_expired", 409)),
            None => Err(invalid(
                "next was issued for another action, account or inputs, or has expired",
            )),
        }
    }
}

fn well_formed(handle: &str) -> bool {
    handle
        .strip_prefix(PREFIX)
        .is_some_and(|hex| hex.len() == 32 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn invalid(message: &str) -> AppError {
    let mut error = AppError::api("pagination_invalid", 409);
    error.msg = message.into();
    error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn handles_are_bound_to_one_call() {
        let cursors = Cursors::open(Store::in_memory().unwrap()).unwrap();
        let handle = cursors.issue("call-a", "raw-cursor").await.unwrap();
        assert!(handle.starts_with("ic_") && handle.len() == 35);
        assert_eq!(
            cursors.resolve(&handle, "call-a").await.unwrap(),
            "raw-cursor"
        );
        let other = cursors.resolve(&handle, "call-b").await.unwrap_err();
        assert_eq!(other.public_code, "pagination_invalid");
        let bogus = cursors.resolve("raw-cursor", "call-a").await.unwrap_err();
        assert_eq!(bogus.public_code, "pagination_invalid");
        assert!(bogus.msg.contains("not a handle"));
    }

    #[tokio::test]
    async fn only_the_newest_handles_are_kept() {
        let cursors = Cursors::open(Store::in_memory().unwrap()).unwrap();
        let first = cursors.issue("call", "0").await.unwrap();
        for i in 1..=HANDLE_CAP {
            cursors.issue("call", &i.to_string()).await.unwrap();
        }
        assert_eq!(
            cursors
                .resolve(&first, "call")
                .await
                .unwrap_err()
                .public_code,
            "pagination_invalid"
        );
    }
}
