//! Single-use approvals for changes. An approval is bound to the digest of the
//! exact request (site, action, account, inputs, spec hash, credential hash),
//! expires after five minutes and is consumed before any network request.
//!
//! Derived from Hycli `src/policy.rs` (`ExecutionRequest`, `Approval`, approval
//! methods of `Policy`), Apache-2.0. Changes: request pacing moved to the network
//! layer (`itda_net::Gate`), so this store only holds approvals.
use itda_types::{
    error::{AppError, AppResult},
    util,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path, sync::Mutex, time::Duration};

const APPROVAL_TTL_MS: i64 = 5 * 60 * 1000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionRequest {
    pub site_id: String,
    pub action_id: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, String>,
    pub spec_hash: String,
    #[serde(default)]
    pub credential_hash: String,
}
impl ExecutionRequest {
    pub fn digest(&self) -> AppResult<String> {
        Ok(util::hash(&serde_json::to_vec(self)?))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReviewInput {
    pub label: String,
    pub value: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Approval {
    pub id: String,
    pub site_id: String,
    pub site_title: String,
    pub action_id: String,
    pub action_title: String,
    pub account_id: String,
    pub account_label: String,
    pub inputs: Vec<ReviewInput>,
    pub expires_at: String,
    pub status: String,
    #[serde(skip)]
    pub request: Option<ExecutionRequest>,
}

pub struct Approvals {
    conn: Mutex<Connection>,
}
impl Approvals {
    pub fn open(path: &Path) -> AppResult<Self> {
        if let Some(parent) = path.parent() {
            util::private_dir(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS approvals(id TEXT PRIMARY KEY, digest TEXT NOT NULL, request TEXT NOT NULL, review TEXT NOT NULL, state TEXT NOT NULL, expires_ms INTEGER NOT NULL);")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
    pub fn request_approval(
        &self,
        request: ExecutionRequest,
        site_title: String,
        action_title: String,
        account_label: String,
        inputs: Vec<ReviewInput>,
    ) -> AppResult<Approval> {
        let expires_ms = util::unix_ms() + APPROVAL_TTL_MS;
        let review = Approval {
            id: util::id(),
            site_id: request.site_id.clone(),
            site_title,
            action_id: request.action_id.clone(),
            action_title,
            account_id: request.account_id.clone(),
            account_label,
            inputs,
            expires_at: chrono::DateTime::from_timestamp_millis(expires_ms)
                .expect("current timestamp")
                .to_rfc3339(),
            status: "pending".into(),
            request: Some(request.clone()),
        };
        let conn = self
            .conn
            .lock()
            .map_err(|_| AppError::api("internal", 500))?;
        conn.execute("INSERT INTO approvals(id,digest,request,review,state,expires_ms) VALUES(?1,?2,?3,?4,'pending',?5)", params![review.id, request.digest()?, serde_json::to_string(&request)?, serde_json::to_string(&review)?, expires_ms])?;
        Ok(review)
    }
    pub fn pending(&self) -> AppResult<Vec<Approval>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| AppError::api("internal", 500))?;
        let mut statement = conn.prepare(
            "SELECT review FROM approvals WHERE state='pending' AND expires_ms>?1 ORDER BY rowid",
        )?;
        let rows = statement.query_map([util::unix_ms()], |r| r.get::<_, String>(0))?;
        let mut result = Vec::new();
        for row in rows {
            result.push(serde_json::from_str(&row?)?);
        }
        Ok(result)
    }
    pub fn review(&self, id: &str) -> AppResult<Approval> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| AppError::api("internal", 500))?;
        let row: Option<(String, String, String, i64)> = conn
            .query_row(
                "SELECT review, request, state, expires_ms FROM approvals WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let (review, request, state, expires) =
            row.ok_or_else(|| AppError::api("not_found", 404))?;
        if state != "pending" || expires <= util::unix_ms() {
            return Err(AppError::api("approval_expired", 409));
        }
        let mut review: Approval = serde_json::from_str(&review)?;
        review.request = Some(serde_json::from_str(&request)?);
        Ok(review)
    }
    /// Call only from the handler that authenticated the approving user.
    /// Consumption happens before any network request, including failed writes.
    pub fn consume(&self, id: &str, request: &ExecutionRequest) -> AppResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| AppError::api("internal", 500))?;
        let affected = conn.execute("UPDATE approvals SET state='consumed' WHERE id=?1 AND digest=?2 AND state IN ('pending','approved') AND expires_ms>?3", params![id, request.digest()?, util::unix_ms()])?;
        if affected != 1 {
            return Err(AppError::api("approval_expired", 409));
        }
        Ok(())
    }
    pub fn reject(&self, id: &str) -> AppResult<()> {
        self.conn
            .lock()
            .map_err(|_| AppError::api("internal", 500))?
            .execute(
                "UPDATE approvals SET state='rejected' WHERE id=?1 AND state='pending'",
                [id],
            )?;
        Ok(())
    }
    pub fn claim(&self, id: &str) -> AppResult<()> {
        let changed=self.conn.lock().map_err(|_|AppError::api("internal",500))?.execute("UPDATE approvals SET state='approved' WHERE id=?1 AND state='pending' AND expires_ms>?2",params![id,util::unix_ms()])?;
        if changed != 1 {
            return Err(AppError::api("approval_expired", 409));
        }
        Ok(())
    }
    pub fn invalidate(&self, site: Option<&str>, account: Option<&str>) -> AppResult<()> {
        self.conn.lock().map_err(|_|AppError::api("internal",500))?.execute("UPDATE approvals SET state='rejected' WHERE state IN ('pending','approved') AND ((?1 IS NOT NULL AND json_extract(request,'$.site_id')=?1) OR (?2 IS NOT NULL AND json_extract(request,'$.account_id')=?2))",params![site,account])?;
        Ok(())
    }
}
