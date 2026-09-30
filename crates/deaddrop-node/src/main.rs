use std::net::SocketAddr;
use std::process::ExitCode;

use deaddrop_node::{DEFAULT_LISTEN, router};
use deaddrop_store::{FilesystemArtifactStore, SqliteMessageStore};

const USAGE: &str =
    "usage: deaddrop-node --db <path> --artifacts <path> [--listen <loopback-address>]";

struct Args {
    db: String,
    artifacts: String,
    listen: SocketAddr,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut db = None;
    let mut artifacts = None;
    let mut listen = None;
    let mut iter = args.iter();

    while let Some(flag) = iter.next() {
        let slot = match flag.as_str() {
            "--db" => &mut db,
            "--artifacts" => &mut artifacts,
            "--listen" => &mut listen,
            other => return Err(format!("unexpected argument {other:?}")),
        };

        let value = iter
            .next()
            .ok_or_else(|| format!("missing value for {flag}"))?;

        if slot.replace(value.clone()).is_some() {
            return Err(format!("duplicate {flag}"));
        }
    }

    let db = db.ok_or("missing required --db")?;
    let artifacts = artifacts.ok_or("missing required --artifacts")?;
    let listen = listen.unwrap_or_else(|| DEFAULT_LISTEN.to_owned());

    let listen: SocketAddr = listen
        .parse()
        .map_err(|error| format!("invalid --listen {listen:?}: {error}"))?;

    // Public binding is out of scope for this node; refuse rather than
    // silently exposing it.
    if !listen.ip().is_loopback() {
        return Err(format!("refusing non-loopback --listen {listen}"));
    }

    Ok(Args {
        db,
        artifacts,
        listen,
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let args = match parse_args(&args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("error: {message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let store = match SqliteMessageStore::open(&args.db) {
        Ok(store) => store,
        Err(error) => {
            eprintln!("error: failed to open store {:?}: {error}", args.db);
            return ExitCode::FAILURE;
        }
    };

    let artifacts = match FilesystemArtifactStore::open(&args.artifacts) {
        Ok(store) => store,
        Err(error) => {
            eprintln!(
                "error: failed to open artifact store {:?}: {error}",
                args.artifacts
            );
            return ExitCode::FAILURE;
        }
    };

    let listener = match tokio::net::TcpListener::bind(args.listen).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("error: failed to bind {}: {error}", args.listen);
            return ExitCode::FAILURE;
        }
    };

    eprintln!("deaddrop-node listening on http://{}", args.listen);

    if let Err(error) = axum::serve(listener, router(store, artifacts)).await {
        eprintln!("error: server failed: {error}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
