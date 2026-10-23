use crate::{Gate, Request};
use itda_types::{error::AppResult, util};
use rusqlite::{OptionalExtension, params};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Transport {
    Direct,
    Fetch,
    Render,
}

impl Transport {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Fetch => "fetch",
            Self::Render => "render",
        }
    }

    fn ttl(self) -> i64 {
        match self {
            Self::Direct => 300_000,
            Self::Fetch | Self::Render => 180_000,
        }
    }
}

#[derive(Clone)]
pub(crate) struct StrategyKey {
    pub(crate) origin: String,
    pub(crate) account: String,
    method: String,
    path: String,
    digest: String,
    contract: String,
}

impl StrategyKey {
    pub(crate) fn new(input: &Request, url: &url::Url, contract: &str) -> AppResult<Self> {
        let digest = util::hash(&serde_json::to_vec(&(
            contract,
            url.as_str(),
            &input.headers,
            &input.body,
        ))?);
        Ok(Self {
            origin: url.origin().ascii_serialization(),
            account: input.account_id.clone(),
            method: input.method.clone(),
            path: url.path().into(),
            digest,
            contract: if contract.is_empty() {
                format!("{}|{}", input.method, url.path())
            } else {
                contract.to_owned()
            },
        })
    }
}

pub(crate) struct Strategy {
    failed: Vec<String>,
    preference: Option<String>,
}

impl Strategy {
    pub(crate) fn routes(&self, render: bool) -> Vec<Transport> {
        let mut routes = vec![Transport::Direct, Transport::Fetch];
        if render {
            routes.push(Transport::Render);
        }
        routes.retain(|route| !self.failed.iter().any(|name| name == route.name()));
        if let Some(preferred) = &self.preference
            && let Some(index) = routes.iter().position(|route| route.name() == preferred)
        {
            routes.swap(0, index);
        }
        routes
    }
}

impl Gate {
    /// Forget a pause that direct-HTTP mode left for this site and account
    /// after a block or challenge: adaptive mode tries the browser instead.
    /// Sign-in pauses and rate-limit cooldowns stay.
    pub(crate) async fn clear_challenge_pause(&self, key: &StrategyKey) -> AppResult<()> {
        let (host, account) = (
            format!("{}|*", key.origin),
            format!("{}|{}", key.origin, key.account),
        );
        self.store
            .call(move |conn| {
                conn.execute(
                    "UPDATE request_gate SET paused='' WHERE paused=?1 AND scope IN (?2,?3)",
                    params![crate::gate::BLOCKED, host, account],
                )?;
                Ok(())
            })
            .await
    }

    pub(crate) async fn strategy(&self, key: &StrategyKey) -> AppResult<Strategy> {
        let key = key.clone();
        let now = self.now();
        self.store
            .call(move |conn| {
                let mut statement = conn.prepare(
                    "SELECT transport FROM strategy_gate
                 WHERE origin=?1 AND account=?2 AND method=?3 AND path=?4 AND digest=?5
                 AND succeeded=0 AND expires>?6",
                )?;
                let failed = statement
                    .query_map(
                        params![
                            key.origin,
                            key.account,
                            key.method,
                            key.path,
                            key.digest,
                            now
                        ],
                        |row| row.get::<_, String>(0),
                    )?
                    .collect::<Result<Vec<_>, _>>()?;
                let preference = conn
                    .query_row(
                        "SELECT transport FROM transport_preference
                 WHERE origin=?1 AND account=?2 AND contract=?3 AND expires>?4",
                        params![key.origin, key.account, key.contract, now],
                        |row| row.get(0),
                    )
                    .optional()?;
                Ok(Strategy { failed, preference })
            })
            .await
    }

    pub(crate) async fn record_strategy(
        &self,
        key: &StrategyKey,
        transport: Transport,
        succeeded: bool,
    ) -> AppResult<()> {
        let key = key.clone();
        let now = self.now();
        self.store.call(move |conn| {
            let tx = conn.transaction()?;
            tx.execute("DELETE FROM strategy_gate WHERE expires<=?1", [now])?;
            tx.execute("DELETE FROM transport_preference WHERE expires<=?1", [now])?;
            tx.execute(
                "INSERT INTO strategy_gate(origin,account,method,path,digest,transport,succeeded,expires)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(origin,account,method,path,digest,transport)
                 DO UPDATE SET succeeded=excluded.succeeded,expires=excluded.expires",
                params![key.origin, key.account, key.method, key.path, key.digest,
                    transport.name(), succeeded, now + transport.ttl()],
            )?;
            if succeeded {
                tx.execute(
                    "INSERT INTO transport_preference(origin,account,contract,transport,expires)
                     VALUES(?1,?2,?3,?4,?5) ON CONFLICT(origin,account,contract)
                     DO UPDATE SET expires=CASE WHEN transport_preference.transport=excluded.transport
                     THEN transport_preference.expires ELSE excluded.expires END,
                     transport=excluded.transport",
                    params![key.origin, key.account, key.contract, transport.name(), now + 3_600_000],
                )?;
            }
            tx.commit()?;
            Ok(())
        }).await
    }
}
