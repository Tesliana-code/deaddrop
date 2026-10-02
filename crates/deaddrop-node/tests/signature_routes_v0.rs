//! Additive detached-signature routes.
//!
//! `POST /v0/messages/{id}/signatures` stores one canonical SignatureV0 whose
//! `message_id` equals `{id}`. `GET /v0/messages/{id}/signatures` returns
//! every stored record as its canonical JSON string. The node never verifies
//! signatures: it is a mailbox, not an identity authority.

use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use deaddrop_node::router;
use deaddrop_protocol::{
    Ed25519PublicKey, MessageId, NodeId, SignatureV0, decode_signature_v0, encode_signature_v0,
};
use deaddrop_store::{FilesystemArtifactStore, SqliteMessageStore};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn app(name: &str) -> Router {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("deaddrop-node-signature-tests")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    router(
        SqliteMessageStore::open(dir.join("messages.sqlite3")).unwrap(),
        FilesystemArtifactStore::open(dir.join("artifacts")).unwrap(),
    )
}

fn record(message: &str, sig: u8) -> SignatureV0 {
    SignatureV0::new(
        MessageId::parse(message).unwrap(),
        NodeId::parse("node-a:shell:deaddrop").unwrap(),
        Ed25519PublicKey::from_bytes([1; 32]),
        [sig; 64],
    )
}

fn canonical(record: &SignatureV0) -> String {
    String::from_utf8(encode_signature_v0(record).unwrap()).unwrap()
}

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}

async fn post(app: &Router, id: &str, body: &str) -> (StatusCode, serde_json::Value) {
    let request = Request::post(format!("/v0/messages/{id}/signatures"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap();
    send(app, request).await
}

async fn list(app: &Router, id: &str) -> (StatusCode, serde_json::Value) {
    let request = Request::get(format!("/v0/messages/{id}/signatures"))
        .body(Body::empty())
        .unwrap();
    send(app, request).await
}

#[tokio::test]
async fn post_is_created_then_exact_replay_is_already_present() {
    let app = app("replay");
    let body = canonical(&record("msg-1", 2));
    let (status, json) = post(&app, "msg-1", &body).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(json["status"], "stored");
    let (status, json) = post(&app, "msg-1", &body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "already_present");
}

#[tokio::test]
async fn distinct_records_are_all_listed_as_canonical_strings() {
    let app = app("list");
    let first = record("msg-1", 2);
    let second = record("msg-1", 3);
    assert_eq!(
        post(&app, "msg-1", &canonical(&second)).await.0,
        StatusCode::CREATED
    );
    assert_eq!(
        post(&app, "msg-1", &canonical(&first)).await.0,
        StatusCode::CREATED
    );

    let (status, json) = list(&app, "msg-1").await;
    assert_eq!(status, StatusCode::OK);
    let listed: Vec<SignatureV0> = json["signatures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| decode_signature_v0(s.as_str().unwrap().as_bytes()).unwrap())
        .collect();
    assert_eq!(listed, vec![first, second]);
}

#[tokio::test]
async fn unknown_message_lists_no_signatures() {
    let app = app("empty");
    let (status, json) = list(&app, "msg-none").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json, serde_json::json!({ "signatures": [] }));
}

#[tokio::test]
async fn path_must_match_record_message_id() {
    let app = app("mismatch");
    let (status, json) = post(&app, "msg-2", &canonical(&record("msg-1", 2))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["error"], "message_id_mismatch");
    assert_eq!(
        list(&app, "msg-1").await.1,
        serde_json::json!({ "signatures": [] })
    );
}

#[tokio::test]
async fn non_canonical_or_wrong_content_type_is_rejected() {
    let app = app("invalid");
    let body = canonical(&record("msg-1", 2));
    let spaced = body.replace(r#"","signer""#, r#"", "signer""#);
    assert_eq!(
        post(&app, "msg-1", &spaced).await.0,
        StatusCode::BAD_REQUEST
    );

    let request = Request::post("/v0/messages/msg-1/signatures")
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(body))
        .unwrap();
    assert_eq!(
        send(&app, request).await.0,
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
}
