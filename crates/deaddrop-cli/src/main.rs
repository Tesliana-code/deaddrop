//! Local Deaddrop node CLI.
//!
//! A consumer of existing protocol and store authority: envelopes are parsed
//! and emitted only through the canonical V0 wire codec and persisted only
//! through SqliteMessageStore.

use std::io::{self, Read, Write};
use std::process::ExitCode;

use deaddrop_protocol::{
    MAX_CANONICAL_ENVELOPE_BYTES, MessageId, decode_envelope_v0, encode_envelope_v0,
};
use deaddrop_store::{
    MessageStore, MessageStoreOutcome, SqliteMessageStore, SqliteMessageStoreError,
};

const EXIT_FAILURE: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_INVALID: u8 = 3;
const EXIT_CONFLICT: u8 = 4;
const EXIT_NOT_FOUND: u8 = 5;

const USAGE: &str = "\
usage:
  deaddrop validate-envelope                      < envelope.json
  deaddrop store-envelope --db <path>             < envelope.json
  deaddrop get-envelope --db <path> --id <message-id>";

struct Failure {
    code: u8,
    message: String,
}

impl Failure {
    fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            eprintln!("error: {}", failure.message);
            if failure.code == EXIT_USAGE {
                eprintln!("{USAGE}");
            }
            ExitCode::from(failure.code)
        }
    }
}

fn run(args: &[String]) -> Result<(), Failure> {
    let Some((command, rest)) = args.split_first() else {
        return Err(Failure::new(EXIT_USAGE, "missing command"));
    };

    match command.as_str() {
        "validate-envelope" => {
            parse_flags(rest, &[])?;
            validate_envelope()
        }
        "store-envelope" => {
            let flags = parse_flags(rest, &["--db"])?;
            store_envelope(&flags[0])
        }
        "get-envelope" => {
            let flags = parse_flags(rest, &["--db", "--id"])?;
            get_envelope(&flags[0], &flags[1])
        }
        other => Err(Failure::new(
            EXIT_USAGE,
            format!("unknown command {other:?}"),
        )),
    }
}

/// Parse exactly the required `--flag value` pairs, in any order, once each.
fn parse_flags(args: &[String], required: &[&str]) -> Result<Vec<String>, Failure> {
    let mut values: Vec<Option<String>> = vec![None; required.len()];
    let mut iter = args.iter();

    while let Some(flag) = iter.next() {
        let Some(index) = required.iter().position(|name| name == flag) else {
            return Err(Failure::new(
                EXIT_USAGE,
                format!("unexpected argument {flag:?}"),
            ));
        };

        let Some(value) = iter.next() else {
            return Err(Failure::new(
                EXIT_USAGE,
                format!("missing value for {flag}"),
            ));
        };

        if values[index].replace(value.clone()).is_some() {
            return Err(Failure::new(EXIT_USAGE, format!("duplicate {flag}")));
        }
    }

    required
        .iter()
        .zip(values)
        .map(|(name, value)| {
            value.ok_or_else(|| Failure::new(EXIT_USAGE, format!("missing required {name}")))
        })
        .collect()
}

/// Read at most one byte past the V0 envelope limit, so oversize input is
/// rejected without buffering all of it.
fn read_stdin() -> Result<Vec<u8>, Failure> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(MAX_CANONICAL_ENVELOPE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Failure::new(EXIT_FAILURE, format!("failed to read stdin: {error}")))?;

    if bytes.len() > MAX_CANONICAL_ENVELOPE_BYTES {
        return Err(Failure::new(
            EXIT_INVALID,
            format!("invalid envelope: input exceeds {MAX_CANONICAL_ENVELOPE_BYTES} bytes"),
        ));
    }

    Ok(bytes)
}

fn decode_stdin() -> Result<deaddrop_protocol::EnvelopeV0, Failure> {
    let bytes = read_stdin()?;
    decode_envelope_v0(&bytes)
        .map_err(|error| Failure::new(EXIT_INVALID, format!("invalid envelope: {error}")))
}

fn open_store(db: &str) -> Result<SqliteMessageStore, Failure> {
    SqliteMessageStore::open(db).map_err(|error| {
        Failure::new(
            EXIT_FAILURE,
            format!("failed to open store {db:?}: {error}"),
        )
    })
}

fn validate_envelope() -> Result<(), Failure> {
    let envelope = decode_stdin()?;
    println!("valid {}", envelope.id());
    Ok(())
}

fn store_envelope(db: &str) -> Result<(), Failure> {
    let envelope = decode_stdin()?;
    let message_id = envelope.id().clone();
    let mut store = open_store(db)?;

    let outcome = store.store(envelope).map_err(|error| match error {
        SqliteMessageStoreError::MessageIdentityConflict { .. } => {
            Failure::new(EXIT_CONFLICT, error.to_string())
        }
        other => Failure::new(EXIT_FAILURE, format!("store failed: {other}")),
    })?;

    let label = match outcome {
        MessageStoreOutcome::Stored => "stored",
        MessageStoreOutcome::AlreadyPresent => "already-present",
    };

    println!("{label} {message_id}");
    Ok(())
}

fn get_envelope(db: &str, id: &str) -> Result<(), Failure> {
    let message_id = MessageId::parse(id).map_err(|error| {
        Failure::new(EXIT_INVALID, format!("invalid message id {id:?}: {error}"))
    })?;

    let store = open_store(db)?;

    let envelope = store
        .load(&message_id)
        .map_err(|error| Failure::new(EXIT_FAILURE, format!("load failed: {error}")))?
        .ok_or_else(|| Failure::new(EXIT_NOT_FOUND, format!("message {message_id} not found")))?;

    let bytes = encode_envelope_v0(&envelope)
        .map_err(|error| Failure::new(EXIT_FAILURE, format!("encode failed: {error}")))?;

    let mut stdout = io::stdout().lock();
    stdout
        .write_all(&bytes)
        .and_then(|()| stdout.flush())
        .map_err(|error| Failure::new(EXIT_FAILURE, format!("failed to write stdout: {error}")))
}
