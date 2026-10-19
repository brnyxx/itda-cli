//! Single-use approvals for changes. An approval is bound to the digest of the
//! exact request (site, action, account, inputs, spec hash, credential hash),
//! expires after five minutes and is consumed before any network request.
//!
//! Derived from Hycli `src/policy.rs` (`ExecutionRequest`, `Approval`, approval
//! methods of `Policy`), Apache-2.0. Changes: request pacing moved to the network
//! layer (`itda_net::Gate`), so this store only holds approvals, and it runs on
//! the shared `itda-store` thread.
use itda_store::{Migration, Store};
use itda_types::{
    error::{AppError, AppResult},
    util,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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

static SCHEMA: Migration = Migration {
    component: "approvals",
    steps: &[
        "CREATE TABLE approvals(id TEXT PRIMARY KEY, digest TEXT NOT NULL, request TEXT NOT NULL, review TEXT NOT NULL, state TEXT NOT NULL, expires_ms INTEGER NOT NULL);",
    ],
};

pub struct Approvals {
    store: Store,
}
impl Approvals {
    pub fn open(store: Store) -> AppResult<Self> {
        store.migrate(&SCHEMA)?;
        Ok(Self { store })
    }
    pub async fn request_approval(
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
        let row = (
            review.id.clone(),
            request.digest()?,
            serde_json::to_string(&request)?,
            serde_json::to_string(&review)?,
        );
        self.store
            .call(move |conn| {
                conn.execute("INSERT INTO approvals(id,digest,request,review,state,expires_ms) VALUES(?1,?2,?3,?4,'pending',?5)", params![row.0, row.1, row.2, row.3, expires_ms])?;
                Ok(())
            })
            .await?;
        Ok(review)
    }
    pub async fn pending(&self) -> AppResult<Vec<Approval>> {
        self.store
            .call(|conn| {
                let mut statement = conn.prepare(
                    "SELECT review FROM approvals WHERE state='pending' AND expires_ms>?1 ORDER BY rowid",
                )?;
                let rows = statement.query_map([util::unix_ms()], |r| r.get::<_, String>(0))?;
                let mut result = Vec::new();
                for row in rows {
                    result.push(serde_json::from_str(&row?)?);
                }
                Ok(result)
            })
            .await
    }
    pub async fn review(&self, id: &str) -> AppResult<Approval> {
        let id = id.to_owned();
        let row: Option<(String, String, String, i64)> = self
            .store
            .call(move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT review, request, state, expires_ms FROM approvals WHERE id=?1",
                        [id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                    )
                    .optional()?)
            })
            .await?;
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
    pub async fn consume(&self, id: &str, request: &ExecutionRequest) -> AppResult<()> {
        let (id, digest) = (id.to_owned(), request.digest()?);
        self.changed_once(move |conn| {
            Ok(conn.execute("UPDATE approvals SET state='consumed' WHERE id=?1 AND digest=?2 AND state IN ('pending','approved') AND expires_ms>?3", params![id, digest, util::unix_ms()])?)
        })
        .await
    }
    pub async fn reject(&self, id: &str) -> AppResult<()> {
        let id = id.to_owned();
        self.store
            .call(move |conn| {
                conn.execute(
                    "UPDATE approvals SET state='rejected' WHERE id=?1 AND state='pending'",
                    [id],
                )?;
                Ok(())
            })
            .await
    }
    pub async fn claim(&self, id: &str) -> AppResult<()> {
        let id = id.to_owned();
        self.changed_once(move |conn| {
            Ok(conn.execute("UPDATE approvals SET state='approved' WHERE id=?1 AND state='pending' AND expires_ms>?2", params![id, util::unix_ms()])?)
        })
        .await
    }
    pub async fn invalidate(&self, site: Option<&str>, account: Option<&str>) -> AppResult<()> {
        let (site, account) = (site.map(str::to_owned), account.map(str::to_owned));
        self.store
            .call(move |conn| {
                conn.execute("UPDATE approvals SET state='rejected' WHERE state IN ('pending','approved') AND ((?1 IS NOT NULL AND json_extract(request,'$.site_id')=?1) OR (?2 IS NOT NULL AND json_extract(request,'$.account_id')=?2))", params![site, account])?;
                Ok(())
            })
            .await
    }

    /// Run an update that must change exactly one pending approval.
    async fn changed_once<F>(&self, update: F) -> AppResult<()>
    where
        F: FnOnce(&mut Connection) -> AppResult<usize> + Send + 'static,
    {
        if self.store.call(update).await? != 1 {
            return Err(AppError::api("approval_expired", 409));
        }
        Ok(())
    }
}
