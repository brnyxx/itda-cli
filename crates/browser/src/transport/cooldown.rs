use super::{OriginCooldown, credentials::action_url};
use itda_types::error::{AppError, AppResult};
use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) fn now_ms() -> i64 {
    millis(SystemTime::now())
}

fn millis(time: SystemTime) -> i64 {
    i64::try_from(
        time.duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

pub(super) fn seed(
    cooldowns: &mut BTreeMap<String, i64>,
    snapshot: &[OriginCooldown],
    now: i64,
) -> AppResult<()> {
    cooldowns.retain(|_, until| *until > now);
    for limit in snapshot {
        let origin = action_url(&limit.origin)?.origin().ascii_serialization();
        if origin != limit.origin {
            return Err(AppError::api("bad_url", 400));
        }
        if limit.until_ms > now {
            let until = cooldowns.entry(origin).or_insert(limit.until_ms);
            *until = (*until).max(limit.until_ms);
        }
    }
    Ok(())
}

pub(super) fn retry_until(status: u16, headers: &[(String, String)], now: i64) -> Option<i64> {
    headers
        .iter()
        .filter(|(name, _)| name == "retry-after")
        .filter_map(|(_, value)| {
            if let Ok(seconds) = value.trim().parse::<u64>() {
                return (seconds > 0).then(|| {
                    now.saturating_add(
                        i64::try_from(seconds)
                            .unwrap_or(i64::MAX)
                            .saturating_mul(1000),
                    )
                });
            }
            httpdate::parse_http_date(value.trim())
                .ok()
                .map(millis)
                .filter(|until| *until > now)
        })
        .max()
        .or_else(|| (status == 429).then(|| now.saturating_add(60_000)))
}

#[cfg(test)]
mod tests {
    use super::{retry_until, seed};
    use crate::transport::OriginCooldown;
    use std::{
        collections::BTreeMap,
        time::{Duration, UNIX_EPOCH},
    };

    #[test]
    fn snapshots_only_extend_future_canonical_origins() {
        let mut state = BTreeMap::from([
            ("https://expired.test".into(), 1000),
            ("https://kept.test".into(), 5000),
        ]);
        seed(
            &mut state,
            &[
                OriginCooldown {
                    origin: "https://kept.test".into(),
                    until_ms: 2000,
                },
                OriginCooldown {
                    origin: "https://new.test".into(),
                    until_ms: 3000,
                },
                OriginCooldown {
                    origin: "https://expired.test".into(),
                    until_ms: 999,
                },
                OriginCooldown {
                    origin: "https://new.test".into(),
                    until_ms: 4000,
                },
            ],
            1000,
        )
        .unwrap();
        assert_eq!(
            state,
            BTreeMap::from([
                ("https://kept.test".into(), 5000),
                ("https://new.test".into(), 4000),
            ])
        );
        for origin in [
            "https://user:secret@example.test",
            "https://example.test?secret=value",
            "https://example.test/path",
            "https://example.test#fragment",
            "https://EXAMPLE.test",
            "file:///tmp/example",
        ] {
            assert!(
                seed(
                    &mut state,
                    &[OriginCooldown {
                        origin: origin.into(),
                        until_ms: 5000,
                    }],
                    1000
                )
                .is_err(),
                "{origin}"
            );
        }
        seed(&mut state, &[], 5000).unwrap();
        assert!(state.is_empty());
    }

    #[test]
    fn provider_delay_and_dates_use_absolute_milliseconds() {
        let now = 1_700_000_000_123;
        let headers = |value: &str| vec![("retry-after".into(), value.into())];
        assert_eq!(retry_until(429, &[], now), Some(now + 60_000));
        assert_eq!(retry_until(200, &headers("120"), now), Some(now + 120_000));
        assert_eq!(retry_until(200, &headers("0"), now), None);
        assert_eq!(retry_until(200, &headers("invalid"), now), None);
        let date = httpdate::fmt_http_date(UNIX_EPOCH + Duration::from_secs(1_700_000_180));
        assert_eq!(
            retry_until(302, &headers(&date), now),
            Some(1_700_000_180_000)
        );
        assert_eq!(retry_until(200, &headers(&date), 1_700_000_181_000), None);
        assert_eq!(
            retry_until(429, &headers("18446744073709551615"), now),
            Some(i64::MAX)
        );
    }
}
