//! Local Deaddrop HTTP node.
//!
//! A network boundary over existing authority, not a second protocol
//! implementation: request bodies are decoded only by `decode_envelope_v0`,
//! response envelopes are produced only by `encode_envelope_v0`, and
//! persistence goes only through `SqliteMessageStore`.
//!
//! A `201 Created` means only that this local node accepted and durably
//! stored the envelope. It is not delivery evidence of any kind, and no
//! delivery event is recorded.
//!
//! `GET /v0/messages?to=<node-id>` lists the ids of locally stored envelopes
//! addressed exactly to that recipient. It returns ids only; each envelope is
//! fetched through `GET /v0/messages/{id}`. The listing records nothing (no
//! read, ack, or claim state), and its order is enumeration order only, never
//! semantic, causal, chronological, delivery, or priority order. Like every
//! route here it assumes the loopback-only local node boundary.
//!
//! Artifacts are opaque exact bytes. Identity, layout, and integrity
//! verification belong to `ArtifactRef` and `FilesystemArtifactStore`; this
//! adapter only moves bytes and maps outcomes to HTTP. Artifact routes never
//! touch the message store.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use deaddrop_protocol::{ArtifactRef, MessageId, NodeId, decode_envelope_v0, encode_envelope_v0};
use deaddrop_store::{
    ArtifactPutOutcome, ArtifactStore, FilesystemArtifactStore, MessageStore, MessageStoreOutcome,
    SqliteMessageStore, SqliteMessageStoreError,
};
use serde_json::json;

/// Local implementation safety limit on request body size.
///
/// Non-normative: this protects this node process only. It is not an
/// EnvelopeV0 size law and other implementations need not share it.
pub const LOCAL_BODY_LIMIT_BYTES: usize = 1024 * 1024;

/// Local implementation safety limit on artifact upload body size.
///
/// Non-normative: artifacts have no Deaddrop protocol size law. This exists
/// only because this V0 HTTP adapter buffers the complete artifact request
/// body in memory before handing it to the store. Applies only to
/// `POST /v0/artifacts`; message routes keep `LOCAL_BODY_LIMIT_BYTES`.
pub const LOCAL_ARTIFACT_BODY_LIMIT_BYTES: usize = 16 * 1024 * 1024;

/// Default listen address. Loopback only.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:8787";

/// Minimal async adapter around the synchronous stores. Calls run on the
/// blocking pool, serialized per store by its own mutex.
#[derive(Clone)]
struct NodeState {
    messages: Arc<Mutex<SqliteMessageStore>>,
    artifacts: Arc<Mutex<FilesystemArtifactStore>>,
}

impl NodeState {
    async fn with_store<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut SqliteMessageStore) -> T + Send + 'static,
    ) -> Result<T, Response> {
        let store = Arc::clone(&self.messages);

        tokio::task::spawn_blocking(move || {
            let mut store = store
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            operation(&mut store)
        })
        .await
        .map_err(|error| internal(format!("store task failed: {error}")))
    }

    async fn with_artifacts<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut FilesystemArtifactStore) -> T + Send + 'static,
    ) -> Result<T, Response> {
        let store = Arc::clone(&self.artifacts);

        tokio::task::spawn_blocking(move || {
            let mut store = store
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            operation(&mut store)
        })
        .await
        .map_err(|_| internal("artifact store task failed".to_owned()))
    }
}

pub fn router(messages: SqliteMessageStore, artifacts: FilesystemArtifactStore) -> Router {
    let state = NodeState {
        messages: Arc::new(Mutex::new(messages)),
        artifacts: Arc::new(Mutex::new(artifacts)),
    };

    let messages = Router::new()
        .route("/v0/messages", post(post_message).get(list_messages))
        .route("/v0/messages/{id}", get(get_message))
        .layer(DefaultBodyLimit::max(LOCAL_BODY_LIMIT_BYTES));

    let artifacts = Router::new()
        .route("/v0/artifacts", post(post_artifact))
        .route("/v0/artifacts/{artifact_ref}", get(get_artifact))
        .layer(DefaultBodyLimit::max(LOCAL_ARTIFACT_BODY_LIMIT_BYTES));

    messages.merge(artifacts).with_state(state)
}

fn error_response(status: StatusCode, error: &str, detail: String) -> Response {
    (
        status,
        axum::Json(json!({ "status": "error", "error": error, "detail": detail })),
    )
        .into_response()
}

fn internal(detail: String) -> Response {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal", detail)
}

fn has_content_type(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case(expected))
}

async fn post_message(State(state): State<NodeState>, headers: HeaderMap, body: Bytes) -> Response {
    if !has_content_type(&headers, "application/json") {
        return error_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "expected Content-Type: application/json".to_owned(),
        );
    }

    let envelope = match decode_envelope_v0(&body) {
        Ok(envelope) => envelope,
        Err(error) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_envelope",
                error.to_string(),
            );
        }
    };

    let message_id = envelope.id().as_str().to_owned();

    let outcome = match state.with_store(move |store| store.store(envelope)).await {
        Ok(outcome) => outcome,
        Err(response) => return response,
    };

    match outcome {
        Ok(MessageStoreOutcome::Stored) => (
            StatusCode::CREATED,
            axum::Json(json!({ "status": "stored", "message_id": message_id })),
        )
            .into_response(),
        Ok(MessageStoreOutcome::AlreadyPresent) => (
            StatusCode::OK,
            axum::Json(json!({ "status": "already_present", "message_id": message_id })),
        )
            .into_response(),
        Err(error @ SqliteMessageStoreError::MessageIdentityConflict { .. }) => {
            error_response(StatusCode::CONFLICT, "identity_conflict", error.to_string())
        }
        Err(error) => internal(format!("store failed: {error}")),
    }
}

/// Exactly one query parameter, `to`, is accepted. Anything else (missing,
/// repeated, or additional parameters, or bytes that do not decode to UTF-8)
/// is rejected rather than guessed at.
async fn list_messages(State(state): State<NodeState>, RawQuery(query): RawQuery) -> Response {
    let Some(value) = query.as_deref().and_then(single_to_param) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_query",
            "expected exactly one query parameter: to=<node-id>".to_owned(),
        );
    };

    let recipient = match NodeId::parse(value.clone()) {
        Ok(recipient) => recipient,
        Err(error) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_recipient",
                format!("{value:?}: {error}"),
            );
        }
    };

    let listed = match state
        .with_store(move |store| store.ids_for_recipient(&recipient))
        .await
    {
        Ok(listed) => listed,
        Err(response) => return response,
    };

    match listed {
        Ok(ids) => {
            let message_ids: Vec<&str> = ids.iter().map(MessageId::as_str).collect();
            (
                StatusCode::OK,
                axum::Json(json!({ "message_ids": message_ids })),
            )
                .into_response()
        }
        Err(error) => internal(format!("list failed: {error}")),
    }
}

/// `to=<value>` as the only pair, decoded per
/// `application/x-www-form-urlencoded` (`+` is a space, `%XX` is a byte).
/// Unlike lossy form decoders, invalid UTF-8 is an error, never U+FFFD.
fn single_to_param(query: &str) -> Option<String> {
    let value = query.strip_prefix("to=")?;

    if value.contains(['&', '=']) {
        return None;
    }

    let mut bytes = Vec::with_capacity(value.len());
    let mut input = value.bytes();

    while let Some(byte) = input.next() {
        bytes.push(match byte {
            b'+' => b' ',
            b'%' => {
                let high = char::from(input.next()?).to_digit(16)?;
                let low = char::from(input.next()?).to_digit(16)?;
                (high * 16 + low) as u8
            }
            other => other,
        });
    }

    String::from_utf8(bytes).ok()
}

async fn get_message(State(state): State<NodeState>, Path(id): Path<String>) -> Response {
    let message_id = match MessageId::parse(id.clone()) {
        Ok(message_id) => message_id,
        Err(error) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_message_id",
                format!("{id:?}: {error}"),
            );
        }
    };

    let loaded = match state.with_store(move |store| store.load(&message_id)).await {
        Ok(loaded) => loaded,
        Err(response) => return response,
    };

    let envelope = match loaded {
        Ok(Some(envelope)) => envelope,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                format!("message {id} not found"),
            );
        }
        Err(error) => return internal(format!("load failed: {error}")),
    };

    match encode_envelope_v0(&envelope) {
        Ok(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            bytes,
        )
            .into_response(),
        Err(error) => internal(format!("encode failed: {error}")),
    }
}

// Artifact store errors carry local filesystem paths; they are deliberately
// not echoed into HTTP responses.

async fn post_artifact(
    State(state): State<NodeState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !has_content_type(&headers, "application/octet-stream") {
        return error_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "expected Content-Type: application/octet-stream".to_owned(),
        );
    }

    let outcome = match state.with_artifacts(move |store| store.put(&body)).await {
        Ok(outcome) => outcome,
        Err(response) => return response,
    };

    match outcome {
        Ok(ArtifactPutOutcome::Stored(artifact)) => (
            StatusCode::CREATED,
            axum::Json(json!({ "status": "stored", "artifact_ref": artifact.to_string() })),
        )
            .into_response(),
        Ok(ArtifactPutOutcome::AlreadyPresent(artifact)) => (
            StatusCode::OK,
            axum::Json(json!({
                "status": "already_present",
                "artifact_ref": artifact.to_string(),
            })),
        )
            .into_response(),
        Err(_) => internal("artifact store failed".to_owned()),
    }
}

async fn get_artifact(
    State(state): State<NodeState>,
    Path(artifact_ref): Path<String>,
) -> Response {
    let artifact: ArtifactRef = match artifact_ref.parse() {
        Ok(artifact) => artifact,
        Err(error) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_artifact_ref",
                error.to_string(),
            );
        }
    };

    let loaded = match state
        .with_artifacts(move |store| store.get(&artifact))
        .await
    {
        Ok(loaded) => loaded,
        Err(response) => return response,
    };

    match loaded {
        Ok(Some(bytes)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/octet-stream")],
            bytes,
        )
            .into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "artifact not found".to_owned(),
        ),
        Err(_) => internal("artifact store failed".to_owned()),
    }
}
