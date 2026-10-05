//! The durable task journal: one append-only JSONL file per task.
//!
//! ```text
//! <home>/tasks/T-d62f78.jsonl
//! {"v":1,"task":"T-d62f78","n":1,"records":[{"task_accepted":{…}},{"plan_validated":{…}}]}
//! {"v":1,"task":"T-d62f78","n":2,"records":[{"wire":{"event":{…}}},{"step_assigned":{…}},…]}
//! ```
//!
//! One line is one commit: every record of one transition (Agent Wire's
//! ASSIGN and the step's `step_assigned`, say), so a crash can never leave
//! half a transition on disk.
//!
//! The journal is the task's source of truth: a [`crate::run::TaskRun`] is
//! nothing but the replay of its records, and Agent Wire's events are kept
//! verbatim (`wire`) so the orchestrator re-validates them on every replay.
//! Every record is written, and synced, before anything it implies is sent:
//! a record that is not on disk was never acted on.
//!
//! Identity: a commit is `(task, n)`, `n` counting from 1 with no gaps; a
//! record is its 1-based position across all commits. Reading is strict:
//!
//! - a final line with no newline is a torn write: discarded, and cut off
//!   before the next append (it was never acted on);
//! - a line repeating the previous one byte for byte is an append retried
//!   after a crash: skipped;
//! - any other bad line — not JSON, an unknown record type, a wrong task,
//!   a gap or reordering — is an error, never skipped, because skipping it
//!   could change the task's state;
//! - a version other than [`VERSION`] is an error: fail closed.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use agent_wire_coordinate::Event;
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;

/// Where task journals live, under a node home.
pub fn dir(home: &Path) -> PathBuf {
    home.join("tasks")
}

/// One task's journal file.
pub fn path(home: &Path, task: &str) -> PathBuf {
    dir(home).join(format!("{task}.jsonl"))
}

/// A validated step as recorded: its capability already resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedStep {
    pub id: String,
    pub capability: String,
    pub worker: String,
    /// read-only | report-only, as resolved at acceptance. Historical:
    /// dispatch is authorized against the current context, never this.
    pub mode: String,
    pub objective: String,
    pub depends_on: Vec<String>,
    pub request: String,
}

/// Which memory a task's plan was informed by: present only when the human
/// asked with `/task::wire --recall`. Refs only, never recalled text.
/// Historical provenance: it authorizes nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecallUsed {
    /// What the orchestrator could read: `room/<name>`.
    pub scope: String,
    /// `episode/<task id>` of every episode the planner was shown, in order.
    pub episodes: Vec<String>,
}

/// One durable task transition. There is no record a worker writes: every
/// one is the orchestrator's, or Agent Wire's own event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Record {
    /// The human's task, accepted. `human_message` is the room message
    /// carrying your words; `accepted_message` the wire's acceptance line.
    TaskAccepted {
        room: String,
        objective: String,
        human_message: String,
        accepted_message: String,
        at_ms: u64,
        /// Absent (and not written) for a task planned without recall.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recall: Option<RecallUsed>,
    },
    /// The plan as validated: every capability resolved to its worker.
    PlanValidated {
        steps: Vec<PlannedStep>,
    },
    /// One Agent Wire event, exactly as Agent Wire recorded it.
    Wire {
        event: Event,
    },
    /// A step assigned (Agent Wire ASSIGN at `wire_seq`) and its room
    /// request `request` carrying `text`.
    StepAssigned {
        step: String,
        worker: String,
        request: String,
        wire_seq: u64,
        at_ms: u64,
        text: String,
    },
    /// The assignee's room report, recorded by Agent Wire. `excerpt` is
    /// what synthesis may be handed; the full text stays in signed history.
    ReportReceived {
        step: String,
        report: String,
        from: String,
        status: String,
        excerpt: String,
        at_ms: u64,
    },
    /// A report Agent Wire refused (not the assignee): nothing recorded.
    ReportRejected {
        step: String,
        report: String,
        from: String,
        reason: String,
    },
    /// Agent Wire's COMPLETE for the step, at `wire_seq`.
    StepEvaluated {
        step: String,
        wire_seq: u64,
    },
    StepFailed {
        step: String,
        reason: String,
        at_ms: u64,
    },
    StepBlocked {
        step: String,
        reason: String,
    },
    TaskCompleted {
        at_ms: u64,
    },
    TaskFailed {
        reason: String,
        at_ms: u64,
    },
    /// The wire's outcome line, sent as room message `message`.
    Announced {
        message: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    v: u32,
    task: String,
    n: u64,
    records: Vec<Record>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalError {
    Io(String),
    /// A complete line that cannot be a record of this journal.
    Corrupt {
        line: usize,
        reason: String,
    },
    UnknownVersion {
        line: usize,
        version: u64,
    },
    /// The records parse but do not replay into a legal task.
    Replay {
        record: usize,
        reason: String,
    },
}

impl std::fmt::Display for JournalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Corrupt { line, reason } => write!(f, "line {line}: corrupt ({reason})"),
            Self::UnknownVersion { line, version } => {
                write!(f, "line {line}: unknown journal version {version}")
            }
            Self::Replay { record, reason } => write!(f, "record {record}: {reason}"),
        }
    }
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> JournalError + '_ {
    move |e| JournalError::Io(format!("{}: {e}", path.display()))
}

/// What was read: the records, and how many bytes of the file they span
/// (anything after is a torn final write).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub records: Vec<Record>,
    pub commits: u64,
    pub valid_len: u64,
    pub torn: bool,
}

/// Parse a journal's bytes, strictly (see the module docs).
pub fn parse(task: &str, bytes: &[u8]) -> Result<Loaded, JournalError> {
    let valid_len = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let mut records = Vec::new();
    let mut commits = 0u64;
    let mut previous: Option<&[u8]> = None;
    // Complete lines only: everything up to, not including, the last newline.
    let complete = &bytes[..valid_len.saturating_sub(1)];
    let lines = (valid_len > 0)
        .then(|| complete.split(|&b| b == b'\n'))
        .into_iter()
        .flatten();
    for (index, line) in lines.enumerate() {
        let number = index + 1;
        if line.is_empty() {
            return Err(JournalError::Corrupt {
                line: number,
                reason: "empty line".into(),
            });
        }
        if previous == Some(line) {
            continue;
        }
        let corrupt = |reason: String| JournalError::Corrupt {
            line: number,
            reason,
        };
        let value: serde_json::Value =
            serde_json::from_slice(line).map_err(|e| corrupt(e.to_string()))?;
        match value.get("v").and_then(serde_json::Value::as_u64) {
            Some(v) if v == u64::from(VERSION) => {}
            Some(version) => {
                return Err(JournalError::UnknownVersion {
                    line: number,
                    version,
                });
            }
            None => return Err(corrupt("no version".into())),
        }
        let entry: Entry = serde_json::from_value(value).map_err(|e| corrupt(e.to_string()))?;
        if entry.task != task {
            return Err(corrupt(format!("record of task {}", entry.task)));
        }
        commits += 1;
        if entry.n != commits {
            return Err(corrupt(format!(
                "commit {} where {commits} was due",
                entry.n
            )));
        }
        if entry.records.is_empty() {
            return Err(corrupt("empty commit".into()));
        }
        records.extend(entry.records);
        previous = Some(line);
    }
    Ok(Loaded {
        records,
        commits,
        valid_len: valid_len as u64,
        torn: valid_len < bytes.len(),
    })
}

/// Read a journal without changing it.
pub fn read(path: &Path, task: &str) -> Result<Loaded, JournalError> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|mut f| f.read_to_end(&mut bytes))
        .map_err(io(path))?;
    parse(task, &bytes)
}

/// Every task journal under `home`, by task id: (task, path).
pub fn list(home: &Path) -> Result<Vec<(String, PathBuf)>, JournalError> {
    let dir = dir(home);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io(&dir)(e)),
    };
    let mut out = Vec::new();
    for entry in entries {
        let path = entry.map_err(io(&dir))?.path();
        let Some(task) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".jsonl"))
        else {
            continue;
        };
        if crate::is_task_id(task) {
            out.push((task.to_owned(), path));
        }
    }
    out.sort();
    Ok(out)
}

/// Make a directory's entries durable: a file created or renamed in it
/// survives a crash only once its directory is synced.
pub fn sync_dir(dir: &Path) -> Result<(), JournalError> {
    File::open(dir).and_then(|d| d.sync_all()).map_err(io(dir))
}

/// `create_dir_all`, then sync the parent of every directory it made.
pub fn create_dir_durable(dir: &Path) -> Result<(), JournalError> {
    if dir.is_dir() {
        return Ok(());
    }
    if let Some(parent) = dir.parent() {
        create_dir_durable(parent)?;
    }
    match std::fs::create_dir(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(e) => return Err(io(dir)(e)),
    }
    match dir.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => sync_dir(parent),
        _ => Ok(()),
    }
}

/// An injected storage failure, for tests: the journal's appends succeed
/// `n` more times, then each one writes half its line and fails, as a
/// crash or a full disk would. Shared by clones.
#[derive(Debug, Clone)]
pub struct Fault(Arc<AtomicUsize>);

impl Fault {
    pub fn after(successes: usize) -> Self {
        Self(Arc::new(AtomicUsize::new(successes)))
    }

    /// Whether this append may succeed (and count it).
    fn allow(&self) -> bool {
        self.0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
    }
}

/// An open journal, appended to by its one writer.
#[derive(Debug)]
pub struct Journal {
    path: PathBuf,
    task: String,
    file: File,
    /// Records on disk.
    written: usize,
    commits: u64,
    /// A failed append may have left a partial line: nothing more is
    /// appended in this process. Reopening cuts it off.
    poisoned: bool,
    fault: Option<Fault>,
}

impl Journal {
    /// Start a new task's journal with its first records. Fails if the
    /// task already has one: a task id is never reused.
    ///
    /// Durable in full before `Ok`: the first commit is synced, then the
    /// directory, so the file itself survives a crash. On failure the file
    /// is removed: nothing it held was acted on.
    pub fn create(home: &Path, task: &str, records: &[Record]) -> Result<Self, JournalError> {
        Self::create_with(home, task, records, None)
    }

    /// [`Journal::create`] with an injected [`Fault`].
    pub fn create_with(
        home: &Path,
        task: &str,
        records: &[Record],
        fault: Option<Fault>,
    ) -> Result<Self, JournalError> {
        let dir = dir(home);
        create_dir_durable(&dir)?;
        let path = path(home, task);
        let file = OpenOptions::new()
            .append(true)
            .create_new(true)
            .open(&path)
            .map_err(io(&path))?;
        let mut journal = Self {
            path,
            task: task.to_owned(),
            file,
            written: 0,
            commits: 0,
            poisoned: false,
            fault,
        };
        let made = journal.append(records).and_then(|()| sync_dir(&dir));
        if let Err(e) = made {
            let _ = std::fs::remove_file(&journal.path);
            let _ = sync_dir(&dir);
            return Err(e);
        }
        Ok(journal)
    }

    /// Open an existing journal to continue it. A torn final write is cut
    /// off first, so the next record starts on a line of its own.
    pub fn open(path: &Path, task: &str) -> Result<(Self, Loaded), JournalError> {
        let loaded = read(path, task)?;
        let file = OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(io(path))?;
        if loaded.torn {
            file.set_len(loaded.valid_len).map_err(io(path))?;
            file.sync_all().map_err(io(path))?;
        }
        Ok((
            Self {
                path: path.to_owned(),
                task: task.to_owned(),
                file,
                written: loaded.records.len(),
                commits: loaded.commits,
                poisoned: false,
                fault: None,
            },
            loaded,
        ))
    }

    /// Inject a [`Fault`] into this journal's appends.
    pub fn with_fault(mut self, fault: Option<Fault>) -> Self {
        self.fault = fault;
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Records on disk.
    pub fn written(&self) -> usize {
        self.written
    }

    /// Append `records` as one commit, and sync. Only after `Ok` may
    /// anything they imply be sent.
    pub fn append(&mut self, records: &[Record]) -> Result<(), JournalError> {
        if records.is_empty() {
            return Ok(());
        }
        if self.poisoned {
            return Err(JournalError::Io(format!(
                "{}: an earlier append failed; reopen to continue",
                self.path.display()
            )));
        }
        let entry = Entry {
            v: VERSION,
            task: self.task.clone(),
            n: self.commits + 1,
            records: records.to_vec(),
        };
        let mut bytes = serde_json::to_vec(&entry).map_err(|e| JournalError::Io(e.to_string()))?;
        bytes.push(b'\n');
        let path = self.path.clone();
        // Poisoned until the line is known durable.
        self.poisoned = true;
        if let Some(fault) = &self.fault
            && !fault.allow()
        {
            let _ = self.file.write_all(&bytes[..bytes.len() / 2]);
            let _ = self.file.sync_data();
            return Err(JournalError::Io(format!(
                "{}: injected write failure",
                path.display()
            )));
        }
        self.file.write_all(&bytes).map_err(io(&path))?;
        self.file.sync_data().map_err(io(&path))?;
        self.poisoned = false;
        self.written += records.len();
        self.commits += 1;
        Ok(())
    }

    /// Append whatever of `all` is not on disk yet.
    pub fn catch_up(&mut self, all: &[Record]) -> Result<(), JournalError> {
        let new = all.get(self.written..).unwrap_or_default().to_vec();
        self.append(&new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("deaddrop-journal-{}", std::process::id()))
            .join(test);
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn failed(step: &str) -> Record {
        Record::StepFailed {
            step: step.into(),
            reason: "timeout".into(),
            at_ms: 7,
        }
    }

    fn line(n: u64, records: &[Record]) -> String {
        let entry = Entry {
            v: VERSION,
            task: "T-abc123".into(),
            n,
            records: records.to_vec(),
        };
        format!("{}\n", serde_json::to_string(&entry).unwrap())
    }

    #[test]
    fn recall_provenance_is_written_only_when_used() {
        let accepted = |recall| Record::TaskAccepted {
            room: "d34ddr0p".into(),
            objective: "o".into(),
            human_message: "h".into(),
            accepted_message: "a".into(),
            at_ms: 1,
            recall,
        };
        // Without recall: the record is byte for byte what it always was.
        let plain = serde_json::to_string(&accepted(None)).unwrap();
        assert_eq!(
            plain,
            r#"{"task_accepted":{"room":"d34ddr0p","objective":"o","human_message":"h","accepted_message":"a","at_ms":1}}"#
        );
        assert_eq!(
            serde_json::from_str::<Record>(&plain).unwrap(),
            accepted(None)
        );
        let used = accepted(Some(RecallUsed {
            scope: "room/d34ddr0p".into(),
            episodes: vec!["episode/T-00000a".into()],
        }));
        let text = serde_json::to_string(&used).unwrap();
        assert!(
            text.ends_with(
                r#""recall":{"scope":"room/d34ddr0p","episodes":["episode/T-00000a"]}}}"#
            )
        );
        assert_eq!(serde_json::from_str::<Record>(&text).unwrap(), used);
        // Strict as every record: nothing else rides along.
        let extra = text.replace(r#""episodes""#, r#""text":"x","episodes""#);
        assert!(serde_json::from_str::<Record>(&extra).is_err());
    }

    #[test]
    fn appends_and_reads_back_in_order() {
        let home = home("append");
        let mut j = Journal::create(&home, "T-abc123", &[failed("a")]).unwrap();
        j.append(&[failed("b"), failed("c")]).unwrap();
        j.append(&[]).unwrap();
        let loaded = read(&path(&home, "T-abc123"), "T-abc123").unwrap();
        assert_eq!(loaded.records, [failed("a"), failed("b"), failed("c")]);
        assert_eq!((loaded.commits, loaded.torn), (2, false));
        assert!(
            Journal::create(&home, "T-abc123", &[failed("a")]).is_err(),
            "a task id is never reused"
        );
        let (mut j, _) = Journal::open(&path(&home, "T-abc123"), "T-abc123").unwrap();
        assert_eq!(j.written(), 3);
        j.catch_up(&[failed("a"), failed("b"), failed("c"), failed("d")])
            .unwrap();
        assert_eq!(
            read(&path(&home, "T-abc123"), "T-abc123")
                .unwrap()
                .records
                .len(),
            4
        );
        assert_eq!(list(&home).unwrap()[0].0, "T-abc123");
    }

    #[test]
    fn a_torn_final_commit_is_discarded_whole_and_cut_before_the_next() {
        let home = home("torn");
        Journal::create(&home, "T-abc123", &[failed("a")]).unwrap();
        let p = path(&home, "T-abc123");
        let whole = line(2, &[failed("b"), failed("c")]);
        let mut bytes = std::fs::read(&p).unwrap();
        bytes.extend_from_slice(&whole.as_bytes()[..whole.len() - 1]);
        std::fs::write(&p, &bytes).unwrap();
        let loaded = read(&p, "T-abc123").unwrap();
        assert!(loaded.torn);
        assert_eq!(loaded.records, [failed("a")], "no half commit");
        let (mut j, _) = Journal::open(&p, "T-abc123").unwrap();
        j.append(&[failed("z")]).unwrap();
        let loaded = read(&p, "T-abc123").unwrap();
        assert_eq!(loaded.records, [failed("a"), failed("z")]);
        assert!(!loaded.torn);
    }

    #[test]
    fn a_corrupt_middle_line_is_an_error_never_skipped() {
        let text = format!(
            "{}not json\n{}",
            line(1, &[failed("a")]),
            line(2, &[failed("b")])
        );
        assert!(matches!(
            parse("T-abc123", text.as_bytes()),
            Err(JournalError::Corrupt { line: 2, .. })
        ));
        let text = format!("{}\n{}", line(1, &[failed("a")]), line(2, &[failed("b")]));
        assert!(matches!(
            parse("T-abc123", text.as_bytes()),
            Err(JournalError::Corrupt { line: 2, .. })
        ));
    }

    #[test]
    fn unknown_versions_and_record_types_fail_closed() {
        let v2 = line(1, &[failed("a")]).replace("\"v\":1", "\"v\":2");
        assert_eq!(
            parse("T-abc123", v2.as_bytes()),
            Err(JournalError::UnknownVersion {
                line: 1,
                version: 2
            })
        );
        let unknown = line(1, &[failed("a")]).replace("step_failed", "memory_write");
        assert!(matches!(
            parse("T-abc123", unknown.as_bytes()),
            Err(JournalError::Corrupt { line: 1, .. })
        ));
        let extra = line(1, &[failed("a")]).replace("\"at_ms\":7", "\"at_ms\":7,\"grant\":\"all\"");
        assert!(parse("T-abc123", extra.as_bytes()).is_err());
        assert!(parse("T-other1", line(1, &[failed("a")]).as_bytes()).is_err());
    }

    #[test]
    fn an_injected_failure_leaves_only_the_durable_prefix() {
        let home = home("fault");
        let mut j =
            Journal::create_with(&home, "T-abc123", &[failed("a")], Some(Fault::after(2))).unwrap();
        j.append(&[failed("b")]).unwrap();
        assert!(
            j.append(&[failed("c"), failed("d")]).is_err(),
            "third append fails"
        );
        assert!(j.append(&[failed("e")]).is_err(), "poisoned: nothing after");
        let p = path(&home, "T-abc123");
        let loaded = read(&p, "T-abc123").unwrap();
        assert!(loaded.torn, "half a line was written");
        assert_eq!(loaded.records, [failed("a"), failed("b")]);
        let (mut j, _) = Journal::open(&p, "T-abc123").unwrap();
        j.append(&[failed("z")]).unwrap();
        assert_eq!(
            read(&p, "T-abc123").unwrap().records,
            [failed("a"), failed("b"), failed("z")]
        );
    }

    #[test]
    fn a_journal_that_cannot_be_made_durable_is_not_left_behind() {
        let home = home("fault-create");
        assert!(
            Journal::create_with(&home, "T-abc123", &[failed("a")], Some(Fault::after(0))).is_err()
        );
        assert!(!path(&home, "T-abc123").exists());
        assert!(list(&home).unwrap().is_empty());
    }

    #[test]
    fn directory_sync_is_real_not_a_no_op() {
        let home = home("dirsync");
        let nested = home.join("a/b/c");
        create_dir_durable(&nested).unwrap();
        assert!(nested.is_dir());
        sync_dir(&nested).unwrap();
        assert!(sync_dir(&home.join("missing")).is_err());
    }

    #[test]
    fn a_retried_commit_is_skipped_but_any_other_gap_is_corrupt() {
        let one = line(1, &[failed("a")]);
        let two = line(2, &[failed("b")]);
        let retried = format!("{one}{one}{two}");
        assert_eq!(
            parse("T-abc123", retried.as_bytes()).unwrap().records,
            [failed("a"), failed("b")]
        );
        let reused = format!("{one}{}", line(1, &[failed("b")]));
        assert!(parse("T-abc123", reused.as_bytes()).is_err());
        let gap = format!("{one}{}", line(3, &[failed("b")]));
        assert!(parse("T-abc123", gap.as_bytes()).is_err());
    }
}
