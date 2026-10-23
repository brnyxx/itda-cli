use super::*;
use crate::transport::{BrowserMode, BrowserRequest, proxy::GuardedProxy};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{TcpListener, TcpStream},
    process::Command,
};

#[test]
fn nested_target_configuration_keeps_the_outer_reply() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut ws = tungstenite::accept(stream).unwrap();
        let outer: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        ws.send(Message::text(json!({
            "method":"Target.attachedToTarget","sessionId":"main",
            "params":{"sessionId":"child","targetInfo":{"targetId":"worker","type":"worker"},"waitingForDebugger":true}
        }).to_string())).unwrap();
        ws.send(Message::text(
            json!({"id":outer["id"],"result":{"answer":42}}).to_string(),
        ))
        .unwrap();
        let mut recursively_attached = false;
        loop {
            let command: Value =
                serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
            if command["method"] == "Target.setAutoAttach" {
                recursively_attached = true;
            }
            let resumed = command["method"] == "Runtime.runIfWaitingForDebugger";
            if resumed {
                assert!(recursively_attached);
            }
            ws.send(Message::text(
                json!({"id":command["id"],"result":{}}).to_string(),
            ))
            .unwrap();
            if resumed {
                break;
            }
        }
    });
    let stream = TcpStream::connect(address).unwrap();
    let (socket, _) = tungstenite::client(format!("ws://{address}"), stream).unwrap();
    let mut browser = Browser {
        origin: "http://example.test".into(),
        account: "account".into(),
        child: Command::new("/usr/bin/true").spawn().unwrap(),
        profile: std::env::temp_dir().join(format!("itda-protocol-test-{}", uuid::Uuid::new_v4())),
        socket,
        proxy: GuardedProxy::new().unwrap(),
        next_id: 0,
        waiting: BTreeSet::new(),
        replies: BTreeMap::new(),
        context: None,
        page: None,
        credential_cookies: BTreeMap::new(),
        credential_digest: None,
        cooldowns: BTreeMap::new(),
    };
    let request = BrowserRequest {
        url: "http://example.test/".into(),
        method: "GET".into(),
        headers: vec![],
        body: None,
        account_id: "account".into(),
        credential_fingerprint: None,
        read_only: true,
        limit: 1024,
        timeout: Duration::from_secs(3),
        allow_local: false,
        mode: BrowserMode::Fetch,
        cooldowns: Vec::new(),
    };
    let url = url::Url::parse(&request.url).unwrap();
    let mut call = Call {
        request: &request,
        url: &url,
        deadline: Instant::now() + Duration::from_secs(3),
        session: "main".into(),
        frame: "frame".into(),
        sessions: BTreeSet::from(["main".into()]),
        worker_scripts: BTreeSet::new(),
        bootstrap: None,
        response: None,
        loaded: false,
        settling: false,
        action_started: false,
        action_ids: BTreeSet::new(),
        read_posts: BTreeSet::new(),
        action_network: None,
        extra_expected: 0,
        extra_seen: 0,
        active: BTreeMap::new(),
        received: 0,
        failure: None,
        rate_limited: None,
        response_cookies: vec![],
        resource_limits: BTreeMap::new(),
    };
    let reply = browser
        .command(Some("main"), "Runtime.evaluate", json!({}), &mut call)
        .unwrap();
    assert_eq!(reply["answer"], 42);
    assert!(browser.replies.is_empty());
    server.join().unwrap();
}
