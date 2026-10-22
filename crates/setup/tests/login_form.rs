//! The sign-in through a site's own form, against a local fixture: the form
//! page hands out a token and a cookie, the post must send both back, a good
//! password answers with a redirect and a session cookie, a bad one with the
//! form again. Nothing here touches a keyring.
#![allow(clippy::unwrap_used)]
use axum::{
    Form, Router,
    http::{HeaderMap, StatusCode},
    response::{AppendHeaders, Html, IntoResponse, Redirect},
    routing::get,
};
use itda_net::{Gate, Network};
use itda_setup::login::{Credentials, form};
use itda_store::Store;
use itda_vault::Secret;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

const FORM: &str = r#"<html><head><title>Sign in</title></head><body>
<form action="/session" method="post">
<input type="hidden" name="_csrf" value="token-1">
<input type="hidden" name="next" value="/notices">
<label>User <input name="user_name" autocomplete="username"></label>
<label>Password <input type="password" name="password"></label>
<input type="checkbox" name="remember" checked>
<button type="submit" name="commit" value="Sign in">Sign in</button>
</form></body></html>"#;

async fn serve(posts: Arc<AtomicUsize>) -> String {
    let router = Router::new()
        .route(
            "/login",
            get(|| async { ([("set-cookie", "pre=before; Path=/; HttpOnly")], Html(FORM)) }),
        )
        .route(
            "/session",
            axum::routing::post(
                move |headers: HeaderMap, Form(fields): Form<HashMap<String, String>>| {
                    let posts = posts.clone();
                    async move {
                        posts.fetch_add(1, Ordering::SeqCst);
                        let cookie = headers
                            .get("cookie")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or_default()
                            .to_owned();
                        let expected = [
                            ("_csrf", "token-1"),
                            ("next", "/notices"),
                            ("remember", "on"),
                            ("commit", "Sign in"),
                            ("user_name", "lab"),
                        ];
                        let complete = expected
                            .iter()
                            .all(|(k, v)| fields.get(*k).map(String::as_str) == Some(v));
                        if !complete || !cookie.contains("pre=before") {
                            return (StatusCode::BAD_REQUEST, "incomplete form").into_response();
                        }
                        if fields.get("password").map(String::as_str) != Some("right") {
                            return Html(FORM.replace("<form", "<p>Wrong password</p><form"))
                                .into_response();
                        }
                        (
                            AppendHeaders([
                                ("set-cookie", "pre=; Max-Age=0; Path=/"),
                                ("set-cookie", "SESSIONID=s3cr3t-value; Path=/; HttpOnly"),
                            ]),
                            Redirect::to("/notices"),
                        )
                            .into_response()
                    }
                },
            ),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

fn network() -> Network {
    let dir = std::env::temp_dir().join(format!("itda-setup-{}", itda_types::util::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(&dir.join("itda.db")).unwrap();
    Network::new(Arc::new(Gate::open(store).unwrap()), true)
}

fn credentials(password: &str) -> Credentials {
    Credentials {
        user: "lab".into(),
        password: Secret::new(password),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_form_is_sent_once_with_its_hidden_fields_and_the_session_kept() {
    let posts = Arc::new(AtomicUsize::new(0));
    let base = serve(posts.clone()).await;
    let net = network();
    let scope = url::Url::parse(&base).unwrap();

    let signed = form(
        &net,
        &scope,
        &format!("{base}/login?next=%2Fnotices"),
        None,
        &credentials("right"),
        None,
    )
    .await
    .unwrap();
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    assert_eq!(signed.session.cookies, "SESSIONID=s3cr3t-value");
    assert_eq!(signed.session.count, 1);
    assert_eq!(signed.cookie.as_deref(), Some("SESSIONID"));
    assert_eq!(signed.form.path, "/login?next=%2Fnotices");
    assert_eq!(signed.form.user, "user_name");
    assert_eq!(signed.form.password, "password");

    // The definition names the fields: the same form, found by them.
    let named = form(
        &net,
        &scope,
        &format!("{base}/login"),
        Some(("user_name", "password")),
        &credentials("right"),
        None,
    )
    .await
    .unwrap();
    assert_eq!(named.form.path, "/login");
    assert_eq!(posts.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_password_is_reported_and_never_retried() {
    let posts = Arc::new(AtomicUsize::new(0));
    let base = serve(posts.clone()).await;
    let net = network();
    let scope = url::Url::parse(&base).unwrap();
    let Err(error) = form(
        &net,
        &scope,
        &format!("{base}/login"),
        None,
        &credentials("wrong"),
        None,
    )
    .await
    else {
        panic!("a wrong password signed in");
    };
    assert_eq!(error.public_code, "auth_required");
    assert!(error.msg.contains("did not accept"), "{}", error.msg);
    assert_eq!(posts.load(Ordering::SeqCst), 1);

    // A page without a sign-in form sends no password anywhere.
    let Err(error) = form(
        &net,
        &scope,
        &format!("{base}/notices"),
        None,
        &credentials("right"),
        None,
    )
    .await
    else {
        panic!("a page without a form signed in");
    };
    assert_ne!(error.public_code, "auth_required");
    assert_eq!(posts.load(Ordering::SeqCst), 1);

    // A sign-in page on another site is refused before anything is fetched.
    let Err(error) = form(
        &net,
        &url::Url::parse("http://127.0.0.2:1/").unwrap(),
        &format!("{base}/login"),
        None,
        &credentials("right"),
        None,
    )
    .await
    else {
        panic!("another site's page signed in");
    };
    assert!(error.msg.contains("another site"), "{}", error.msg);
}

/// A site that keeps its session in the address: the form page hands out a
/// token, the sign-in answers with a redirect carrying `sid`, and no cookie.
const QUERY_FORM: &str = r#"<html><head><title>Sign in</title></head><body>
<form action="/session" method="post">
<input type="hidden" name="_csrf" value="token-q">
<input name="user_name">
<input type="password" name="password">
</form></body></html>"#;

async fn serve_query_session() -> String {
    let token = "9f8e7d6c5b4a3210fedc";
    let router = Router::new()
        .route("/login", get(|| async { Html(QUERY_FORM) }))
        .route(
            "/session",
            axum::routing::post(
                move |Form(fields): Form<HashMap<String, String>>| async move {
                    if fields.get("password").map(String::as_str) != Some("right") {
                        return Html(QUERY_FORM.replace("<form", "<p>Wrong</p><form"))
                            .into_response();
                    }
                    Redirect::to(&format!("/portal/main.do?sid={token}")).into_response()
                },
            ),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_the_site_carries_in_the_query_is_captured_from_the_redirect() {
    let base = serve_query_session().await;
    let net = network();
    let scope = url::Url::parse(&base).unwrap();
    let signed = form(
        &net,
        &scope,
        &format!("{base}/login"),
        None,
        &credentials("right"),
        None,
    )
    .await
    .unwrap();
    // No cookie was set; the session is the query the redirect carried.
    assert_eq!(signed.session.count, 0);
    assert_eq!(
        signed
            .session
            .query
            .as_ref()
            .map(|(k, v)| (k.as_str(), v.as_str())),
        Some(("sid", "9f8e7d6c5b4a3210fedc"))
    );
    assert_eq!(signed.session.value_from(None), "query:sid");
    assert_eq!(
        signed.session.secret_for("query:sid").expose(),
        "9f8e7d6c5b4a3210fedc"
    );
}
