use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use deaddrop_node::{LOCAL_BODY_LIMIT_BYTES, router};
use deaddrop_protocol::MessageId;
use deaddrop_store::{DeliveryEventStore, SqliteDeliveryEventStore, SqliteMessageStore};
use http_body_util::BodyExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

const CANONICAL: &str = concat!(
    r#"{"protocol":"deaddrop/0","id":"msg-http-1","#,
    r#""from":"node-a:agent:deaddrop","#,
    r#""to":"node-b:agent:deaddrop","#,
    r#""kind":"handoff","#,
    r#""correlation_id":"corr-http-1","#,
    r#""body":"continue this task","#,
    r#""artifact_refs":["#,
    r#""sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]}"#
);

const CONFLICTING: &str = concat!(
    r#"{"protocol":"deaddrop/0","id":"msg-http-1","#,
    r#""from":"node-a:agent:deaddrop","#,
    r#""to":"node-b:agent:deaddrop","#,
    r#""kind":"handoff","#,
    r#""correlation_id":"corr-http-1","#,
    r#""body":"a different body","#,
    r#""artifact_refs":["#,
    r#""sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]}"#
);

fn fresh_db(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("deaddrop-node-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.sqlite3"));
    let _ = std::fs::remove_file(&path);
    path
}

fn app(db: &PathBuf) -> Router {
    router(SqliteMessageStore::open(db).unwrap())
}

async fn post(app: &Router, body: &str) -> (StatusCode, Vec<u8>) {
    let request = Request::post("/v0/messages")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap();
    send(app, request).await
}

async fn get(app: &Router, id: &str) -> (StatusCode, Vec<u8>) {
    let request = Request::get(format!("/v0/messages/{id}"))
        .body(Body::empty())
        .unwrap();
    send(app, request).await
}

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, Vec<u8>) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, body.to_vec())
}

fn json(body: &[u8]) -> serde_json::Value {
    serde_json::from_slice(body).unwrap()
}

#[tokio::test]
async fn a_post_canonical_is_created_stored() {
    let app = app(&fresh_db("a"));

    let (status, body) = post(&app, CANONICAL).await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        json(&body),
        serde_json::json!({ "status": "stored", "message_id": "msg-http-1" })
    );
}

#[tokio::test]
async fn b_post_exact_replay_is_already_present() {
    let app = app(&fresh_db("b"));

    assert_eq!(post(&app, CANONICAL).await.0, StatusCode::CREATED);
    let (status, body) = post(&app, CANONICAL).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json(&body),
        serde_json::json!({ "status": "already_present", "message_id": "msg-http-1" })
    );
}

#[tokio::test]
async fn c_post_noncanonical_is_bad_request() {
    let app = app(&fresh_db("c"));

    let pretty = serde_json::to_string_pretty(&json(CANONICAL.as_bytes())).unwrap();
    assert_eq!(post(&app, &pretty).await.0, StatusCode::BAD_REQUEST);

    let trailing_newline = format!("{CANONICAL}\n");
    assert_eq!(
        post(&app, &trailing_newline).await.0,
        StatusCode::BAD_REQUEST
    );

    assert_eq!(get(&app, "msg-http-1").await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn d_post_invalid_field_shape_is_bad_request() {
    let app = app(&fresh_db("d"));

    let unknown_field = CANONICAL.replace(r#""body":"#, r#""extra":1,"body":"#);
    let missing_correlation = CANONICAL.replace(r#""correlation_id":"corr-http-1","#, "");
    let unknown_kind = CANONICAL.replace(r#""kind":"handoff""#, r#""kind":"teleport""#);
    let wrong_protocol = CANONICAL.replace("deaddrop/0", "deaddrop/1");

    for body in [
        unknown_field,
        missing_correlation,
        unknown_kind,
        wrong_protocol,
        "not json".to_owned(),
    ] {
        let (status, response) = post(&app, &body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(json(&response)["error"], "invalid_envelope");
    }
}

#[tokio::test]
async fn e_post_same_id_different_envelope_is_conflict() {
    let app = app(&fresh_db("e"));

    assert_eq!(post(&app, CANONICAL).await.0, StatusCode::CREATED);
    let (status, body) = post(&app, CONFLICTING).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json(&body)["error"], "identity_conflict");
    assert_eq!(get(&app, "msg-http-1").await.1, CANONICAL.as_bytes());
}

#[tokio::test]
async fn f_get_existing_returns_exact_canonical_bytes() {
    let app = app(&fresh_db("f"));

    post(&app, CANONICAL).await;

    let request = Request::get("/v0/messages/msg-http-1")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body, CANONICAL.as_bytes());
}

#[tokio::test]
async fn g_get_invalid_message_id_is_bad_request() {
    let app = app(&fresh_db("g"));

    for id in ["%20msg-http-1", "msg-http-1%20", "msg%0A1"] {
        let (status, body) = get(&app, id).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{id}");
        assert_eq!(json(&body)["error"], "invalid_message_id");
    }
}

#[tokio::test]
async fn h_get_missing_message_is_not_found() {
    let app = app(&fresh_db("h"));

    let (status, body) = get(&app, "msg-absent").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json(&body)["error"], "not_found");
}

#[tokio::test]
async fn i_stored_post_survives_reopening_database() {
    let db = fresh_db("i");

    assert_eq!(post(&app(&db), CANONICAL).await.0, StatusCode::CREATED);

    let reopened = app(&db);
    let (status, body) = get(&reopened, "msg-http-1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, CANONICAL.as_bytes());
    assert_eq!(post(&reopened, CANONICAL).await.0, StatusCode::OK);
}

#[tokio::test]
async fn j_successful_post_creates_no_delivery_evidence() {
    let db = fresh_db("j");

    assert_eq!(post(&app(&db), CANONICAL).await.0, StatusCode::CREATED);

    let events = SqliteDeliveryEventStore::open(&db).unwrap();
    let evidence = events
        .load_for_message(&MessageId::parse("msg-http-1").unwrap())
        .unwrap();
    assert!(evidence.is_empty(), "{evidence:?}");
}

#[tokio::test]
async fn post_without_json_content_type_is_rejected() {
    let app = app(&fresh_db("content-type"));

    let request = Request::post("/v0/messages")
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(CANONICAL))
        .unwrap();

    assert_eq!(
        send(&app, request).await.0,
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
}

#[tokio::test]
async fn loopback_socket_smoke() {
    let app = app(&fresh_db("smoke"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let raw = async |request: String| {
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        String::from_utf8(response).unwrap()
    };

    let posted = raw(format!(
        "POST /v0/messages HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{CANONICAL}",
        CANONICAL.len()
    ))
    .await;
    assert!(posted.starts_with("HTTP/1.1 201"), "{posted}");

    let fetched = raw(format!(
        "GET /v0/messages/msg-http-1 HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    ))
    .await;
    assert!(fetched.starts_with("HTTP/1.1 200"), "{fetched}");
    assert!(
        fetched.ends_with(&format!("\r\n\r\n{CANONICAL}")),
        "{fetched}"
    );
}

#[tokio::test]
async fn post_over_local_body_limit_is_payload_too_large_and_not_persisted() {
    let app = app(&fresh_db("body-limit"));

    let oversized = CANONICAL
        .replace("msg-http-1", "msg-http-oversized")
        .replace(
            "continue this task",
            &"x".repeat(LOCAL_BODY_LIMIT_BYTES + 1),
        );
    assert!(oversized.len() > LOCAL_BODY_LIMIT_BYTES);

    assert_eq!(
        post(&app, &oversized).await.0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        get(&app, "msg-http-oversized").await.0,
        StatusCode::NOT_FOUND
    );
}
