//! Network shell V0, real processes: one relay process running the unchanged
//! `deaddrop-node` router (this test binary re-executed) and independent
//! `deaddrop` CLI processes for each node, each with its own home directory,
//! key and local store. Nodes meet only through the relay's HTTP API.
//!
//! Slice 1 bodies are signed but NOT encrypted; the relay is loopback only.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

use serde_json::Value;

const A: &str = "node-a:shell:deaddrop";
const B: &str = "node-b:shell:deaddrop";
const C: &str = "node-c:shell:deaddrop";
const RELAY_DIR_ENV: &str = "DEADDROP_TEST_RELAY_DIR";

/// Relay-process entry point; a no-op unless spawned by `start_relay`.
#[test]
#[ignore = "relay-process entry; spawned by the network shell proof"]
fn relay_process_entry() {
    let Ok(dir) = std::env::var(RELAY_DIR_ENV) else {
        return;
    };
    let dir = PathBuf::from(dir);
    let app = deaddrop_node::router(
        deaddrop_store::SqliteMessageStore::open(dir.join("relay.sqlite3")).unwrap(),
        deaddrop_store::FilesystemArtifactStore::open(dir.join("artifacts")).unwrap(),
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        println!("RELAY_URL=http://{}", listener.local_addr().unwrap());
        axum::serve(listener, app).await.unwrap();
    });
}

/// The relay process; killed on drop.
struct Relay {
    child: Child,
    url: String,
}

impl Drop for Relay {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn scratch(test: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("deaddrop-network-shell")
        .join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn start_relay(dir: &Path) -> Relay {
    let relay_dir = dir.join("relay");
    std::fs::create_dir_all(&relay_dir).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "relay_process_entry", "--ignored", "--nocapture"])
        .env(RELAY_DIR_ENV, &relay_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let url = BufReader::new(stdout)
        .lines()
        .map(Result::unwrap)
        .find_map(|line| line.strip_prefix("RELAY_URL=").map(str::to_owned))
        .expect("relay announced its url");
    Relay { child, url }
}

fn deaddrop(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_deaddrop"))
        .args(args)
        .arg("--home")
        .arg(home)
        .output()
        .unwrap()
}

fn ok(home: &Path, args: &[&str]) -> Value {
    let output = deaddrop(home, args);
    assert!(
        output.status.success(),
        "deaddrop {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Initialise a node home and return its `identity` output.
fn node(dir: &Path, name: &str, relay: &Relay) -> (PathBuf, Value) {
    let home = dir.join(name.split(':').next().unwrap());
    ok(&home, &["init", "--node", name, "--relay", &relay.url]);
    let identity = ok(&home, &["identity"]);
    assert_eq!(identity["node_id"], name);
    (home, identity)
}

fn trust(home: &Path, peer: &Value) {
    ok(
        home,
        &[
            "peer",
            "add",
            peer["node_id"].as_str().unwrap(),
            peer["key"].as_str().unwrap(),
        ],
    );
}

#[test]
fn two_independent_nodes_exchange_signed_message_artifact_and_ack() {
    let dir = scratch("two-nodes");
    let relay = start_relay(&dir);
    let (a, a_id) = node(&dir, A, &relay);
    let (b, b_id) = node(&dir, B, &relay);
    assert_ne!(a_id["key"], b_id["key"]);
    trust(&a, &b_id);
    trust(&b, &a_id);
    assert_eq!(ok(&a, &["peers"])["peers"][0]["node_id"], B);

    // A publishes an immutable artifact and sends a signed message to B.
    let evidence = dir.join("evidence.txt");
    std::fs::write(&evidence, b"immutable evidence v1").unwrap();
    let artifact = ok(&a, &["artifact", "put", evidence.to_str().unwrap()])["artifact_ref"]
        .as_str()
        .unwrap()
        .to_owned();
    let sent = ok(
        &a,
        &[
            "send",
            B,
            "hello from a",
            "--correlation",
            "conversation-1",
            "--artifact",
            &artifact,
        ],
    )["message_id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Not acknowledged yet.
    assert_eq!(ok(&a, &["status", &sent])["acknowledged"], false);

    // B receives asynchronously and verifies.
    let inbox = ok(&b, &["inbox"]);
    assert_eq!(inbox["received"], serde_json::json!([sent]));
    assert_eq!(inbox["rejected"], serde_json::json!([]));
    let message = &inbox["messages"][0];
    assert_eq!(message["id"], sent.as_str());
    assert_eq!(message["from"], A);
    assert_eq!(message["kind"], "message");
    assert_eq!(message["body"], "hello from a");
    assert_eq!(message["correlation_id"], "conversation-1");
    assert_eq!(message["artifact_refs"], serde_json::json!([artifact]));
    assert_eq!(
        ok(&b, &["status", &sent])["events"],
        serde_json::json!([
            { "kind": "recipient_received", "reported_by": B },
            { "kind": "recipient_verified", "reported_by": B },
        ])
    );

    // B fetches the referenced artifact; bytes are hash-checked.
    let fetched = dir.join("fetched.txt");
    ok(
        &b,
        &[
            "artifact",
            "get",
            &artifact,
            "--out",
            fetched.to_str().unwrap(),
        ],
    );
    assert_eq!(std::fs::read(&fetched).unwrap(), b"immutable evidence v1");

    // B acknowledges; A receives the ACK.
    let ack = ok(&b, &["ack", &sent])["ack_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let a_inbox = ok(&a, &["inbox"]);
    assert_eq!(
        a_inbox["acknowledged"],
        serde_json::json!([{ "ack_id": ack, "message_id": sent, "by": B }])
    );
    assert_eq!(a_inbox["messages"], serde_json::json!([]));
    let status = ok(&a, &["status", &sent]);
    assert_eq!(status["acknowledged"], true);
    assert_eq!(
        status["events"],
        serde_json::json!([{ "kind": "recipient_acknowledged", "reported_by": B }])
    );

    // Re-syncing is idempotent on both sides.
    assert_eq!(ok(&b, &["inbox"])["received"], serde_json::json!([]));
    assert_eq!(ok(&a, &["inbox"])["acknowledged"], serde_json::json!([]));
}

#[test]
fn node_rejects_messages_from_an_untrusted_peer() {
    let dir = scratch("untrusted");
    let relay = start_relay(&dir);
    let (_a, a_id) = node(&dir, A, &relay);
    let (b, b_id) = node(&dir, B, &relay);
    let (c, _c_id) = node(&dir, C, &relay);
    trust(&b, &a_id);
    // C knows B, but B never added C.
    trust(&c, &b_id);

    let sent = ok(&c, &["send", B, "let me in"])["message_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let inbox = ok(&b, &["inbox"]);
    assert_eq!(inbox["received"], serde_json::json!([]));
    assert_eq!(inbox["messages"], serde_json::json!([]));
    assert_eq!(
        inbox["rejected"],
        serde_json::json!([{ "message_id": sent, "reason": "untrusted_sender" }])
    );

    // Sending to a peer the sender does not trust fails before the relay.
    let refused = deaddrop(&c, &["send", A, "unknown peer"]);
    assert!(!refused.status.success());
}

#[test]
fn init_refuses_a_non_loopback_relay() {
    let dir = scratch("non-loopback");
    let output = deaddrop(
        &dir.join("a"),
        &[
            "init",
            "--node",
            A,
            "--relay",
            "http://relay.example.org:8787",
        ],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("encryption"));
}
