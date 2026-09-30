use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use deaddrop_node::router;
use deaddrop_protocol::{EnvelopeV0, MessageId, MessageKind, NodeId, encode_envelope_v0};
use deaddrop_store::{FilesystemArtifactStore, SqliteMessageStore};
use http_body_util::BodyExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

fn fresh_db(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("deaddrop-node-discovery-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.sqlite3"));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(path.with_extension("artifacts"));
    path
}

fn app(db: &PathBuf) -> Router {
    router(
        SqliteMessageStore::open(db).unwrap(),
        FilesystemArtifactStore::open(db.with_extension("artifacts")).unwrap(),
    )
}

fn wire(id: &str, to: &str) -> String {
    let envelope = EnvelopeV0::new(
        MessageId::parse(id).unwrap(),
        NodeId::parse("node-a").unwrap(),
        NodeId::parse(to).unwrap(),
        MessageKind::Message,
        format!("body of {id}"),
    );
    String::from_utf8(encode_envelope_v0(&envelope).unwrap()).unwrap()
}

/// Posted in an order deliberately unrelated to the listing order.
fn corpus() -> Vec<String> {
    vec![
        wire("msg-3", "node-b"),
        wire("msg-c", "node-c"),
        wire("msg-1", "node-b"),
        wire("msg-prefix", "node-b-suffix"),
        wire("msg-case", "Node-B"),
        wire("msg-2", "node-b"),
    ]
}

const NODE_B_IDS: &[&str] = &["msg-1", "msg-2", "msg-3"];

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, Vec<u8>) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, body.to_vec())
}

async fn post(app: &Router, body: &str) -> StatusCode {
    let request = Request::post("/v0/messages")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap();
    send(app, request).await.0
}

async fn get(app: &Router, uri: &str) -> (StatusCode, Vec<u8>) {
    send(app, Request::get(uri).body(Body::empty()).unwrap()).await
}

fn json(body: &[u8]) -> serde_json::Value {
    serde_json::from_slice(body).unwrap()
}

fn listing(ids: &[&str]) -> serde_json::Value {
    serde_json::json!({ "message_ids": ids })
}

async fn filled(name: &str) -> Router {
    let app = app(&fresh_db(name));
    for body in corpus() {
        assert_eq!(post(&app, &body).await, StatusCode::CREATED);
    }
    app
}

#[tokio::test]
async fn a_lists_only_exact_recipient_ids() {
    let app = filled("a").await;

    let (status, body) = get(&app, "/v0/messages?to=node-b").await;
    assert_eq!(status, StatusCode::OK);
    // Ids only: the response carries no envelope, sequence, time or status.
    assert_eq!(json(&body), listing(NODE_B_IDS));

    assert_eq!(
        json(&get(&app, "/v0/messages?to=node-c").await.1),
        listing(&["msg-c"])
    );
    assert_eq!(
        json(&get(&app, "/v0/messages?to=Node-B").await.1),
        listing(&["msg-case"])
    );
    assert_eq!(
        json(&get(&app, "/v0/messages?to=node").await.1),
        listing(&[])
    );
}

#[tokio::test]
async fn b_empty_inbox_is_empty_collection() {
    let app = app(&fresh_db("b"));

    let (status, body) = get(&app, "/v0/messages?to=node-b").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&body), listing(&[]));
}

#[tokio::test]
async fn c_every_discovered_id_fetches_its_exact_envelope() {
    let app = filled("c").await;

    for id in NODE_B_IDS {
        let (status, body) = get(&app, &format!("/v0/messages/{id}")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(String::from_utf8(body).unwrap(), wire(id, "node-b"));
    }
}

#[tokio::test]
async fn d_exact_replay_does_not_duplicate_ids() {
    let app = filled("d").await;

    for body in corpus() {
        assert_eq!(post(&app, &body).await, StatusCode::OK);
    }

    assert_eq!(
        json(&get(&app, "/v0/messages?to=node-b").await.1),
        listing(NODE_B_IDS)
    );
}

#[tokio::test]
async fn e_discovery_is_deterministic_and_does_not_mutate_state() {
    let app = filled("e").await;

    let before: Vec<_> = get_all(&app).await;
    let first = get(&app, "/v0/messages?to=node-b").await;
    let second = get(&app, "/v0/messages?to=node-b").await;
    let after: Vec<_> = get_all(&app).await;

    assert_eq!(first, second);
    assert_eq!(before, after);

    // Still exactly the same stored envelopes: replay remains idempotent.
    for body in corpus() {
        assert_eq!(post(&app, &body).await, StatusCode::OK);
    }
}

async fn get_all(app: &Router) -> Vec<(StatusCode, Vec<u8>)> {
    let mut all = Vec::new();
    for id in ["msg-1", "msg-2", "msg-3", "msg-c", "msg-prefix", "msg-case"] {
        all.push(get(app, &format!("/v0/messages/{id}")).await);
    }
    all
}

#[tokio::test]
async fn f_percent_encoded_recipient_matches_exactly() {
    let app = app(&fresh_db("f"));
    assert_eq!(
        post(&app, &wire("msg-enc", "node b:é+x")).await,
        StatusCode::CREATED
    );

    assert_eq!(
        json(&get(&app, "/v0/messages?to=node%20b%3A%C3%A9%2Bx").await.1),
        listing(&["msg-enc"])
    );
    // `+` in a query string decodes to a space, so it names a different node.
    assert_eq!(
        json(&get(&app, "/v0/messages?to=node+b:%C3%A9+x").await.1),
        listing(&[])
    );
}

#[tokio::test]
async fn g_invalid_recipient_is_rejected() {
    let app = filled("g").await;

    for uri in [
        "/v0/messages?to=",
        "/v0/messages?to=%20node-b",
        "/v0/messages?to=node-b%20",
        "/v0/messages?to=node%0Ab",
    ] {
        let (status, body) = get(&app, uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(json(&body)["error"], "invalid_recipient", "{uri}");
    }
}

#[tokio::test]
async fn h_malformed_query_is_rejected() {
    let app = filled("h").await;

    for uri in [
        "/v0/messages",
        "/v0/messages?",
        "/v0/messages?from=node-a",
        "/v0/messages?to=node-b&to=node-c",
        "/v0/messages?to=node-b&limit=1",
        "/v0/messages?to=node-b&",
        "/v0/messages?to=node=b",
        // Invalid UTF-8 and bad escapes fail closed instead of decoding
        // lossily into a different, valid NodeId.
        "/v0/messages?to=%FF",
        "/v0/messages?to=node%C3",
        "/v0/messages?to=node%GG",
        "/v0/messages?to=node%F",
    ] {
        let (status, body) = get(&app, uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(json(&body)["error"], "invalid_query", "{uri}");
    }
}

#[tokio::test]
async fn i_discovery_survives_reopening_database() {
    let db = fresh_db("i");
    {
        let app = app(&db);
        for body in corpus() {
            assert_eq!(post(&app, &body).await, StatusCode::CREATED);
        }
    }

    let reopened = app(&db);
    assert_eq!(
        json(&get(&reopened, "/v0/messages?to=node-b").await.1),
        listing(NODE_B_IDS)
    );
}

#[tokio::test]
async fn j_loopback_socket_discovery() {
    let app = app(&fresh_db("j"));
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

    for body in corpus() {
        let posted = raw(format!(
            "POST /v0/messages HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ))
        .await;
        assert!(posted.starts_with("HTTP/1.1 201"), "{posted}");
    }

    let listed = raw(format!(
        "GET /v0/messages?to=node-b HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    ))
    .await;
    assert!(listed.starts_with("HTTP/1.1 200"), "{listed}");
    let (_, body) = listed.split_once("\r\n\r\n").unwrap();
    assert_eq!(json(body.as_bytes()), listing(NODE_B_IDS));

    for id in NODE_B_IDS {
        let fetched = raw(format!(
            "GET /v0/messages/{id} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
        ))
        .await;
        assert!(fetched.starts_with("HTTP/1.1 200"), "{fetched}");
        assert!(
            fetched.ends_with(&format!("\r\n\r\n{}", wire(id, "node-b"))),
            "{fetched}"
        );
    }
}
