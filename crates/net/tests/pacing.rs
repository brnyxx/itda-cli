//! One request at a time per site, spaced, and a site's own Retry-After
//! honoured once. Spacing is checked on a paused runtime against the gate
//! alone; the socket tests run in real time with the pace set to zero, since
//! a paused runtime jumps its clock ahead whenever a request waits on the
//! network.
#![allow(clippy::unwrap_used)]
use axum::{Router, http::StatusCode, response::IntoResponse, routing::get};
use itda_net::{Gate, Network, Outcome, Request, gate};
use itda_store::Store;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::time::Instant;

const SITE: &str = "https://example.test|*";
const OTHER: &str = "https://other.test|*";

/// A gate that reads the paused runtime's clock, so waits cost no real time.
fn paced(interval_ms: i64) -> Gate {
    let base = Instant::now();
    Gate::with_limits(Store::in_memory().unwrap(), interval_ms, 1000)
        .unwrap()
        .with_clock(move || 1_700_000_000_000 + base.elapsed().as_millis() as i64)
}

async fn take_and_release(gate: &Gate, scope: &str, interval: Option<Duration>) {
    let lease = gate.acquire(scope, interval).await.unwrap();
    lease.release(Outcome::Done).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn two_calls_to_one_site_are_spaced() {
    let gate = paced(1200);
    take_and_release(&gate, SITE, None).await;
    let start = Instant::now();
    take_and_release(&gate, SITE, None).await;
    let gap = start.elapsed();
    assert!(
        gap >= Duration::from_millis(1200) && gap < Duration::from_millis(1400),
        "second call waited {gap:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn two_sites_do_not_wait_for_each_other() {
    let gate = paced(1200);
    take_and_release(&gate, SITE, None).await;
    let start = Instant::now();
    take_and_release(&gate, OTHER, None).await;
    assert_eq!(start.elapsed(), Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn one_request_at_a_time_per_site() {
    let gate = paced(0);
    let held = gate.acquire(SITE, None).await.unwrap();
    let waited = tokio::time::timeout(Duration::from_secs(5), gate.acquire(SITE, None)).await;
    assert!(
        waited.is_err(),
        "a second request went while one was in flight"
    );
    drop(waited);
    held.release(Outcome::Done).await.unwrap();
    gate.acquire(SITE, None).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_site_definition_can_ask_for_more_room_but_not_less() {
    let gate = paced(1200);
    take_and_release(&gate, SITE, Some(Duration::from_secs(3))).await;
    let start = Instant::now();
    take_and_release(&gate, SITE, Some(Duration::from_millis(100))).await;
    assert!(start.elapsed() >= Duration::from_secs(3));
    // The previous call asked for 100 ms; the gate's own pace still applies.
    let start = Instant::now();
    take_and_release(&gate, SITE, None).await;
    assert!(start.elapsed() >= Duration::from_millis(1200));
    // And nothing can ask for more than half a minute.
    take_and_release(&gate, OTHER, Some(Duration::from_secs(3600))).await;
    let start = Instant::now();
    take_and_release(&gate, OTHER, None).await;
    assert!(start.elapsed() <= gate::MAX_INTERVAL + Duration::from_millis(500));
}

#[tokio::test(start_paused = true)]
async fn a_short_cooldown_is_waited_out() {
    let gate = paced(0);
    let lease = gate.acquire(SITE, None).await.unwrap();
    lease
        .release(Outcome::RateLimited(Some(Duration::from_secs(5))))
        .await
        .unwrap();
    let start = Instant::now();
    take_and_release(&gate, SITE, None).await;
    let gap = start.elapsed();
    assert!(
        gap >= Duration::from_secs(5) && gap < Duration::from_millis(5600),
        "waited {gap:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_long_cooldown_fails_at_once() {
    let gate = paced(0);
    let lease = gate.acquire(SITE, None).await.unwrap();
    lease
        .release(Outcome::RateLimited(Some(Duration::from_secs(60))))
        .await
        .unwrap();
    let start = Instant::now();
    let error = gate.acquire(SITE, None).await.unwrap_err();
    assert_eq!(error.public_code, "rate_limited");
    assert_eq!(start.elapsed(), Duration::ZERO);
}

// ----- against a local server, in real time, with the pace set to zero -----

async fn host(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    address
}

fn network() -> Arc<Network> {
    let store = Store::in_memory().unwrap();
    Arc::new(Network::new(
        Arc::new(Gate::with_limits(store, 0, 1000).unwrap()),
        true,
    ))
}

#[tokio::test]
async fn parallel_calls_to_one_site_arrive_one_at_a_time() {
    let arrived = Arc::new(AtomicUsize::new(0));
    let first_arrival = Arc::new(tokio::sync::Notify::new());
    let (go, hold) = tokio::sync::watch::channel(false);
    let base = host(Router::new().route(
        "/slow",
        get({
            let arrived = arrived.clone();
            let first_arrival = first_arrival.clone();
            move || {
                let mut hold = hold.clone();
                async move {
                    arrived.fetch_add(1, Ordering::SeqCst);
                    first_arrival.notify_one();
                    hold.wait_for(|released| *released).await.unwrap();
                    "ok"
                }
            }
        }),
    ))
    .await;
    let net = network();
    let calls: Vec<_> = (0..3)
        .map(|_| {
            let net = net.clone();
            let url = format!("{base}/slow");
            tokio::spawn(async move { net.fetch(Request::read(url)).await.unwrap().status })
        })
        .collect();
    first_arrival.notified().await;
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        arrived.load(Ordering::SeqCst),
        1,
        "a second request reached the site while the first was in flight"
    );
    go.send(true).unwrap();
    for call in calls {
        assert_eq!(call.await.unwrap(), 200);
    }
    assert_eq!(arrived.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn calls_to_different_sites_do_not_wait_for_each_other() {
    let (other_seen, seen) = tokio::sync::watch::channel(false);
    // The first site answers only once the second site has been reached.
    let first = host(Router::new().route(
        "/a",
        get(move || {
            let mut seen = seen.clone();
            async move {
                seen.wait_for(|reached| *reached).await.unwrap();
                "a"
            }
        }),
    ))
    .await;
    let second = host(Router::new().route(
        "/b",
        get(move || {
            other_seen.send(true).unwrap();
            async { "b" }
        }),
    ))
    .await;
    let net = network();
    let both = tokio::time::timeout(
        Duration::from_secs(10),
        futures_util::future::join(
            net.fetch(Request::read(format!("{first}/a"))),
            net.fetch(Request::read(format!("{second}/b"))),
        ),
    )
    .await
    .expect("the second site waited for the first one's turn");
    assert_eq!(both.0.unwrap().bytes, b"a");
    assert_eq!(both.1.unwrap().bytes, b"b");
}

/// A site that answers `status` with `Retry-After: 1` once, then 200, and
/// records when each request arrived.
async fn limited_once(
    status: StatusCode,
    retry_after: &'static str,
) -> (String, Arc<Mutex<Vec<Instant>>>) {
    let arrivals = Arc::new(Mutex::new(Vec::new()));
    let base = host(Router::new().route(
        "/limited",
        get({
            let arrivals = arrivals.clone();
            move || {
                let mut arrivals = arrivals.lock().unwrap();
                arrivals.push(Instant::now());
                let first = arrivals.len() == 1;
                async move {
                    if first {
                        (status, [("retry-after", retry_after)], "slow down").into_response()
                    } else {
                        "welcome".into_response()
                    }
                }
            }
        }),
    ))
    .await;
    (base, arrivals)
}

#[tokio::test]
async fn a_short_retry_after_is_waited_once_for_a_read() {
    for status in [
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::SERVICE_UNAVAILABLE,
    ] {
        let (base, arrivals) = limited_once(status, "1").await;
        let response = network()
            .fetch(Request::read(format!("{base}/limited")))
            .await
            .unwrap();
        assert_eq!(response.status, 200, "{status}: the read was not retried");
        let arrivals = arrivals.lock().unwrap();
        assert_eq!(arrivals.len(), 2);
        let gap = arrivals[1].duration_since(arrivals[0]);
        assert!(
            gap >= Duration::from_secs(1),
            "{status}: retried after {gap:?}"
        );
    }
}

#[tokio::test]
async fn a_long_retry_after_stops_the_read_and_the_next_one() {
    let (base, arrivals) = limited_once(StatusCode::TOO_MANY_REQUESTS, "60").await;
    let net = network();
    let response = net
        .fetch(Request::read(format!("{base}/limited")))
        .await
        .unwrap();
    assert_eq!(response.status, 429);
    assert_eq!(response.defense, "rate-limit");
    assert_eq!(
        net.fetch(Request::read(format!("{base}/limited")))
            .await
            .unwrap_err()
            .public_code,
        "rate_limited"
    );
    assert_eq!(arrivals.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_change_is_never_retried() {
    let (base, arrivals) = limited_once(StatusCode::TOO_MANY_REQUESTS, "1").await;
    let mut change = Request::read(format!("{base}/limited"));
    change.read_only = false;
    let response = network().fetch(change).await.unwrap();
    assert_eq!(response.status, 429);
    assert_eq!(arrivals.lock().unwrap().len(), 1);
}
