use std::path::{Path, PathBuf};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use deaddrop_node::{LOCAL_ARTIFACT_BODY_LIMIT_BYTES, LOCAL_BODY_LIMIT_BYTES, router};
use deaddrop_protocol::{ArtifactRef, MAX_CANONICAL_ENVELOPE_BYTES, MessageId};
use deaddrop_store::{
    ArtifactStore, DeliveryEventStore, FilesystemArtifactStore, SqliteDeliveryEventStore,
    SqliteMessageStore,
};
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
    let _ = std::fs::remove_dir_all(artifacts_dir(&path));
    path
}

fn artifacts_dir(db: &Path) -> PathBuf {
    db.with_extension("artifacts")
}

fn app(db: &PathBuf) -> Router {
    router(
        SqliteMessageStore::open(db).unwrap(),
        FilesystemArtifactStore::open(artifacts_dir(db)).unwrap(),
    )
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

#[test]
fn transport_ceiling_is_at_least_the_protocol_envelope_limit() {
    const { assert!(LOCAL_BODY_LIMIT_BYTES >= MAX_CANONICAL_ENVELOPE_BYTES) };
}

#[tokio::test]
async fn post_over_protocol_limit_under_transport_limit_is_413_and_not_persisted() {
    let app = app(&fresh_db("protocol-limit"));

    let framing = CANONICAL.len() - "continue this task".len();
    let sized = |len: usize| {
        CANONICAL
            .replace("msg-http-1", "msg-http-limit")
            .replace("continue this task", &"x".repeat(len - framing - 4))
    };

    let over = sized(MAX_CANONICAL_ENVELOPE_BYTES + 1);
    assert_eq!(over.len(), MAX_CANONICAL_ENVELOPE_BYTES + 1);
    assert!(over.len() < LOCAL_BODY_LIMIT_BYTES);

    let (status, body) = post(&app, &over).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(json(&body)["error"], "envelope_too_large");
    assert_eq!(get(&app, "msg-http-limit").await.0, StatusCode::NOT_FOUND);

    let exact = sized(MAX_CANONICAL_ENVELOPE_BYTES);
    assert_eq!(exact.len(), MAX_CANONICAL_ENVELOPE_BYTES);
    assert_eq!(post(&app, &exact).await.0, StatusCode::CREATED);
    let (status, fetched) = get(&app, "msg-http-limit").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched, exact.as_bytes());
}

// ---- Artifact HTTP boundary ----

const BINARY: &[u8] = &[0x00, 0xff, 0xfe, 0x80, 0xc3, 0x28, 0x0a, 0x0d, 0x00];

async fn post_artifact(app: &Router, bytes: &[u8]) -> (StatusCode, Vec<u8>) {
    let request = Request::post("/v0/artifacts")
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(bytes.to_vec()))
        .unwrap();
    send(app, request).await
}

async fn get_artifact(app: &Router, artifact_ref: &str) -> (StatusCode, Vec<u8>) {
    let request = Request::get(format!("/v0/artifacts/{artifact_ref}"))
        .body(Body::empty())
        .unwrap();
    send(app, request).await
}

/// Expected ref from the store authority, via an isolated scratch store.
fn expected_ref(name: &str, bytes: &[u8]) -> String {
    let dir = artifacts_dir(&fresh_db(&format!("{name}-expected")));
    let mut store = FilesystemArtifactStore::open(dir).unwrap();
    store.put(bytes).unwrap().artifact().to_string()
}

fn artifact_ref_of(body: &[u8]) -> String {
    json(body)["artifact_ref"].as_str().unwrap().to_owned()
}

fn object_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found
}

#[tokio::test]
async fn artifact_a_post_binary_is_created_with_ref() {
    let app = app(&fresh_db("art-a"));

    let (status, body) = post_artifact(&app, BINARY).await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        json(&body),
        serde_json::json!({ "status": "stored", "artifact_ref": expected_ref("art-a", BINARY) })
    );
}

#[tokio::test]
async fn artifact_b_exact_replay_is_already_present() {
    let app = app(&fresh_db("art-b"));

    let (_, first) = post_artifact(&app, BINARY).await;
    let (status, body) = post_artifact(&app, BINARY).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json(&body),
        serde_json::json!({ "status": "already_present", "artifact_ref": artifact_ref_of(&first) })
    );
}

#[tokio::test]
async fn artifact_c_different_bytes_have_different_refs() {
    let app = app(&fresh_db("art-c"));

    let (_, one) = post_artifact(&app, b"one").await;
    let (status, two) = post_artifact(&app, b"two").await;

    assert_eq!(status, StatusCode::CREATED);
    assert_ne!(artifact_ref_of(&one), artifact_ref_of(&two));
}

#[tokio::test]
async fn artifact_d_get_existing_returns_exact_bytes() {
    let app = app(&fresh_db("art-d"));

    let (_, body) = post_artifact(&app, BINARY).await;
    let request = Request::get(format!("/v0/artifacts/{}", artifact_ref_of(&body)))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/octet-stream"
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(bytes, BINARY);
}

#[tokio::test]
async fn artifact_e_zero_length_round_trips() {
    let app = app(&fresh_db("art-e"));

    let (status, body) = post_artifact(&app, b"").await;
    assert_eq!(status, StatusCode::CREATED);
    let artifact_ref = artifact_ref_of(&body);
    assert_eq!(artifact_ref, expected_ref("art-e", b""));

    let (status, bytes) = get_artifact(&app, &artifact_ref).await;
    assert_eq!(status, StatusCode::OK);
    assert!(bytes.is_empty());
}

#[tokio::test]
async fn artifact_f_non_utf8_bytes_round_trip() {
    let app = app(&fresh_db("art-f"));
    let all_bytes: Vec<u8> = (0..=255u8).rev().cycle().take(4096).collect();
    assert!(std::str::from_utf8(&all_bytes).is_err());

    let (status, body) = post_artifact(&app, &all_bytes).await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, bytes) = get_artifact(&app, &artifact_ref_of(&body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, all_bytes);
}

#[tokio::test]
async fn artifact_g_invalid_ref_is_bad_request() {
    let app = app(&fresh_db("art-g"));

    for bad in [
        "sha512:00",
        "sha256:abcd",
        "sha256:gg00000000000000000000000000000000000000000000000000000000000000",
        "not-a-ref",
    ] {
        let (status, body) = get_artifact(&app, bad).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(json(&body)["error"], "invalid_artifact_ref");
    }
}

#[tokio::test]
async fn artifact_h_valid_missing_ref_is_not_found() {
    let app = app(&fresh_db("art-h"));

    let (status, body) = get_artifact(
        &app,
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json(&body)["error"], "not_found");
}

#[tokio::test]
async fn artifact_i_wrong_or_missing_content_type_is_rejected() {
    let db = fresh_db("art-i");
    let app = app(&db);

    for content_type in [Some("application/json"), Some("text/plain"), None] {
        let mut request = Request::post("/v0/artifacts");
        if let Some(content_type) = content_type {
            request = request.header(header::CONTENT_TYPE, content_type);
        }
        let request = request.body(Body::from(BINARY)).unwrap();

        assert_eq!(
            send(&app, request).await.0,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "{content_type:?}"
        );
    }
    assert!(object_files(&artifacts_dir(&db)).is_empty());
}

#[tokio::test]
async fn artifact_j_k_over_local_limit_is_payload_too_large_and_not_persisted() {
    let db = fresh_db("art-j");
    let app = app(&db);

    let oversized = vec![0x5a_u8; LOCAL_ARTIFACT_BODY_LIMIT_BYTES + 1];
    let (status, _) = post_artifact(&app, &oversized).await;

    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(object_files(&artifacts_dir(&db)).is_empty());
    assert_eq!(
        get_artifact(&app, &expected_ref("art-j", &oversized))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn artifact_limit_is_separate_from_message_limit() {
    let app = app(&fresh_db("art-limits"));

    // Above the message limit, within the artifact limit.
    let bytes = vec![0x11_u8; LOCAL_BODY_LIMIT_BYTES + 1];
    assert!(bytes.len() <= LOCAL_ARTIFACT_BODY_LIMIT_BYTES);

    let (status, body) = post_artifact(&app, &bytes).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(get_artifact(&app, &artifact_ref_of(&body)).await.1, bytes);
}

#[tokio::test]
async fn artifact_l_survives_store_reopen() {
    let db = fresh_db("art-l");

    let (status, body) = post_artifact(&app(&db), BINARY).await;
    assert_eq!(status, StatusCode::CREATED);
    let artifact_ref = artifact_ref_of(&body);

    let reopened = app(&db);
    let (status, bytes) = get_artifact(&reopened, &artifact_ref).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, BINARY);
    assert_eq!(post_artifact(&reopened, BINARY).await.0, StatusCode::OK);
}

fn corrupt(db: &Path, artifact_ref: &str) {
    let store = FilesystemArtifactStore::open(artifacts_dir(db)).unwrap();
    let path = store.path_for(&artifact_ref.parse::<ArtifactRef>().unwrap());
    std::fs::write(path, b"CORRUPTED-BYTES").unwrap();
}

#[tokio::test]
async fn artifact_m_corrupted_get_is_internal_error_without_bytes() {
    let db = fresh_db("art-m");
    let app = app(&db);

    let (_, body) = post_artifact(&app, BINARY).await;
    let artifact_ref = artifact_ref_of(&body);
    corrupt(&db, &artifact_ref);

    let (status, body) = get_artifact(&app, &artifact_ref).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!body.windows(9).any(|w| w == b"CORRUPTED"));
    let text = String::from_utf8(body).unwrap();
    assert!(
        !text.contains(artifacts_dir(&db).to_str().unwrap()),
        "{text}"
    );
    assert_eq!(json(text.as_bytes())["error"], "internal");
}

#[tokio::test]
async fn artifact_n_replay_against_corrupted_artifact_fails_closed() {
    let db = fresh_db("art-n");
    let app = app(&db);

    let (_, body) = post_artifact(&app, BINARY).await;
    corrupt(&db, &artifact_ref_of(&body));

    let (status, body) = post_artifact(&app, BINARY).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let text = String::from_utf8(body).unwrap();
    assert!(!text.contains("already_present"), "{text}");
    assert!(
        !text.contains(artifacts_dir(&db).to_str().unwrap()),
        "{text}"
    );
}

#[tokio::test]
async fn artifact_p_post_creates_no_message_or_delivery_semantics() {
    let db = fresh_db("art-p");
    let app = app(&db);
    let before = std::fs::read(&db).unwrap();

    let (status, body) = post_artifact(&app, CANONICAL.as_bytes()).await;
    assert_eq!(status, StatusCode::CREATED);

    // Envelope-shaped artifact bytes remain opaque: no message is stored.
    assert_eq!(get(&app, "msg-http-1").await.0, StatusCode::NOT_FOUND);
    assert_eq!(std::fs::read(&db).unwrap(), before);
    let events = SqliteDeliveryEventStore::open(&db).unwrap();
    assert!(
        events
            .load_for_message(&MessageId::parse("msg-http-1").unwrap())
            .unwrap()
            .is_empty()
    );
    assert_eq!(object_files(&artifacts_dir(&db)).len(), 1);
    assert!(json(&body).get("message_id").is_none());
}

#[tokio::test]
async fn artifact_q_loopback_socket_smoke() {
    let app = app(&fresh_db("art-smoke"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let raw = async |request: Vec<u8>| {
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream.write_all(&request).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        response
    };

    let mut request = format!(
        "POST /v0/artifacts HTTP/1.1\r\nHost: {addr}\r\n\
         Content-Type: application/octet-stream\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        BINARY.len()
    )
    .into_bytes();
    request.extend_from_slice(BINARY);
    let posted = String::from_utf8(raw(request).await).unwrap();
    assert!(posted.starts_with("HTTP/1.1 201"), "{posted}");
    let artifact_ref = expected_ref("art-smoke", BINARY);
    assert!(posted.contains(&artifact_ref), "{posted}");

    let fetched = raw(format!(
        "GET /v0/artifacts/{artifact_ref} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )
    .into_bytes())
    .await;
    assert!(fetched.starts_with(b"HTTP/1.1 200"));
    let mut expected_tail = b"\r\n\r\n".to_vec();
    expected_tail.extend_from_slice(BINARY);
    assert!(fetched.ends_with(&expected_tail));
}
