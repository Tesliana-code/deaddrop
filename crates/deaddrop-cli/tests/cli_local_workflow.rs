use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

const EXIT_INVALID: i32 = 3;
const EXIT_CONFLICT: i32 = 4;
const EXIT_NOT_FOUND: i32 = 5;

const CANONICAL: &str = concat!(
    r#"{"protocol":"deaddrop/0","id":"msg-cli-1","#,
    r#""from":"node-a:agent:deaddrop","#,
    r#""to":"node-b:agent:deaddrop","#,
    r#""kind":"handoff","#,
    r#""correlation_id":"corr-cli-1","#,
    r#""body":"continue this task","#,
    r#""artifact_refs":["#,
    r#""sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]}"#
);

const CONFLICTING: &str = concat!(
    r#"{"protocol":"deaddrop/0","id":"msg-cli-1","#,
    r#""from":"node-a:agent:deaddrop","#,
    r#""to":"node-b:agent:deaddrop","#,
    r#""kind":"handoff","#,
    r#""correlation_id":"corr-cli-1","#,
    r#""body":"a different body","#,
    r#""artifact_refs":["#,
    r#""sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]}"#
);

fn deaddrop(args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deaddrop"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn deaddrop");

    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin)
        .expect("write stdin");

    child.wait_with_output().expect("wait deaddrop")
}

fn fresh_db(name: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("deaddrop-cli-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.sqlite3"));
    let _ = std::fs::remove_file(&path);
    path.to_str().unwrap().to_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn a_canonical_envelope_validates() {
    let output = deaddrop(&["validate-envelope"], CANONICAL.as_bytes());

    assert!(output.status.success(), "{output:?}");
    assert_eq!(stdout(&output), "valid msg-cli-1\n");
}

#[test]
fn b_pretty_noncanonical_json_fails_validation() {
    let pretty = concat!(
        "{\n",
        "  \"protocol\": \"deaddrop/0\",\n",
        "  \"id\": \"msg-cli-1\",\n",
        "  \"from\": \"node-a:agent:deaddrop\",\n",
        "  \"to\": \"node-b:agent:deaddrop\",\n",
        "  \"kind\": \"handoff\",\n",
        "  \"correlation_id\": \"corr-cli-1\",\n",
        "  \"body\": \"continue this task\",\n",
        "  \"artifact_refs\": [\"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"]\n",
        "}"
    );

    let output = deaddrop(&["validate-envelope"], pretty.as_bytes());

    assert_eq!(output.status.code(), Some(EXIT_INVALID), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("not canonical"));

    let trailing_newline = format!("{CANONICAL}\n");
    let output = deaddrop(&["validate-envelope"], trailing_newline.as_bytes());
    assert_eq!(output.status.code(), Some(EXIT_INVALID), "{output:?}");
}

#[test]
fn c_store_then_get_returns_exact_canonical_bytes() {
    let db = fresh_db("roundtrip");

    let stored = deaddrop(&["store-envelope", "--db", &db], CANONICAL.as_bytes());
    assert!(stored.status.success(), "{stored:?}");
    assert_eq!(stdout(&stored), "stored msg-cli-1\n");

    let loaded = deaddrop(&["get-envelope", "--db", &db, "--id", "msg-cli-1"], b"");
    assert!(loaded.status.success(), "{loaded:?}");
    assert_eq!(loaded.stdout, CANONICAL.as_bytes());
}

#[test]
fn d_exact_replay_is_already_present() {
    let db = fresh_db("replay");

    let first = deaddrop(&["store-envelope", "--db", &db], CANONICAL.as_bytes());
    assert_eq!(stdout(&first), "stored msg-cli-1\n");

    let replay = deaddrop(&["store-envelope", "--db", &db], CANONICAL.as_bytes());
    assert!(replay.status.success(), "{replay:?}");
    assert_eq!(stdout(&replay), "already-present msg-cli-1\n");
}

#[test]
fn e_same_id_different_envelope_fails_closed() {
    let db = fresh_db("conflict");

    let first = deaddrop(&["store-envelope", "--db", &db], CANONICAL.as_bytes());
    assert!(first.status.success(), "{first:?}");

    let conflict = deaddrop(&["store-envelope", "--db", &db], CONFLICTING.as_bytes());
    assert_eq!(conflict.status.code(), Some(EXIT_CONFLICT), "{conflict:?}");
    assert!(conflict.stdout.is_empty());

    let loaded = deaddrop(&["get-envelope", "--db", &db, "--id", "msg-cli-1"], b"");
    assert_eq!(loaded.stdout, CANONICAL.as_bytes());
}

#[test]
fn f_invalid_message_id_on_get_fails() {
    let db = fresh_db("invalid-id");

    for id in ["", " msg-cli-1", "msg\n1"] {
        let output = deaddrop(&["get-envelope", "--db", &db, "--id", id], b"");
        assert_eq!(
            output.status.code(),
            Some(EXIT_INVALID),
            "{id:?}: {output:?}"
        );
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn g_missing_message_id_is_distinct_not_found() {
    let db = fresh_db("missing");

    let output = deaddrop(&["get-envelope", "--db", &db, "--id", "msg-absent"], b"");
    assert_eq!(output.status.code(), Some(EXIT_NOT_FOUND), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not found"));
}

/// Like `deaddrop`, but tolerates the CLI exiting before consuming all input.
fn deaddrop_partial_read(args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_deaddrop"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn deaddrop");

    match child.stdin.take().expect("stdin").write_all(stdin) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {}
        Err(error) => panic!("write stdin: {error}"),
    }

    child.wait_with_output().expect("wait deaddrop")
}

fn canonical_of_len(len: usize) -> String {
    let padding = len - (CANONICAL.len() - "continue this task".len());
    let bytes = CANONICAL.replace("continue this task", &"x".repeat(padding));
    assert_eq!(bytes.len(), len);
    bytes
}

#[test]
fn l_stdin_is_bounded_to_the_protocol_envelope_limit() {
    let max = deaddrop_protocol::MAX_CANONICAL_ENVELOPE_BYTES;

    let exact = deaddrop(&["validate-envelope"], canonical_of_len(max).as_bytes());
    assert!(exact.status.success(), "{exact:?}");

    let over = canonical_of_len(max + 1);
    let output = deaddrop(&["validate-envelope"], over.as_bytes());
    assert_eq!(output.status.code(), Some(EXIT_INVALID), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("exceeds"));

    let db = fresh_db("oversize");
    let output = deaddrop(&["store-envelope", "--db", &db], over.as_bytes());
    assert_eq!(output.status.code(), Some(EXIT_INVALID), "{output:?}");
    let output = deaddrop(&["get-envelope", "--db", &db, "--id", "msg-cli-1"], b"");
    assert_eq!(output.status.code(), Some(EXIT_NOT_FOUND), "{output:?}");

    // Far larger input is rejected after reading only max + 1 bytes.
    let output = deaddrop_partial_read(&["validate-envelope"], &vec![b'x'; 32 * max]);
    assert_eq!(output.status.code(), Some(EXIT_INVALID), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("exceeds"));
}
