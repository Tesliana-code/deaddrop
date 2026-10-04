//! Development-only latency marks. Inactive unless `DEADDROP_TIMING_LOG`
//! names a file; then each mark appends one JSON line: the stage, the
//! message id it belongs to, and a wall-clock time in microseconds (shared
//! across processes on one machine). Never message content.

use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_us() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_micros())
}

pub fn enabled() -> bool {
    std::env::var_os("DEADDROP_TIMING_LOG").is_some()
}

/// Record `stage` for message `id` now.
pub fn mark(stage: &str, id: &str) {
    mark_at(stage, id, now_us());
}

/// Record `stage` for request `id`, done by `who` (a node id).
pub fn mark_by(stage: &str, id: &str, who: &str) {
    let Some(path) = std::env::var_os("DEADDROP_TIMING_LOG") else {
        return;
    };
    let line = format!(
        "{{\"stage\":\"{stage}\",\"id\":\"{id}\",\"who\":\"{who}\",\"us\":{}}}\n",
        now_us()
    );
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

/// Record `stage` for message `id` at `us`.
pub fn mark_at(stage: &str, id: &str, us: u128) {
    let Some(path) = std::env::var_os("DEADDROP_TIMING_LOG") else {
        return;
    };
    let line = format!("{{\"stage\":\"{stage}\",\"id\":\"{id}\",\"us\":{us}}}\n");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}
