//! Memory recall V0: a bounded, explainable, read-only view of episodic
//! memory for a caller that asks for it explicitly.
//!
//! ```text
//! Caller + Query → scope gate → memory::select (journal truth, newest
//! terminal first) → RecallBundle (bounded RecallItems, refs first)
//! ```
//!
//! Recall may inform a decision; it never becomes the decision authority.
//! A bundle is plain data: it adds no trust, policy, capability, route or
//! completion, and nothing in /task::wire reads it. Matching is structured
//! and exact; each item says which filters it matched, never a score.
//!
//! Membership of the skupljen is not access to memory: a trusted peer has a
//! channel and an allowed worker may be asked, but neither reads an episode.
//! V0 has no peer-facing recall; [`Caller::Peer`] is always refused.

use std::path::Path;

use serde::Serialize;

use crate::memory::{self, Episode, Query};

/// Episodes a query returns when it names no `recent` N.
pub const DEFAULT_LIMIT: usize = 5;
/// Most episodes one recall returns, whatever it asks for.
pub const MAX_LIMIT: usize = 20;
/// Characters of the human's objective an item carries.
pub const MAX_OBJECTIVE_CHARS: usize = 240;
/// Characters of a failure or step reason an item carries.
pub const MAX_REASON_CHARS: usize = 160;
/// Capabilities, workers, peers, steps and evidence refs per item.
pub const MAX_LIST: usize = 8;
/// Provenance refs per item; the journal ref is always first.
pub const MAX_PROVENANCE: usize = 12;
/// Bytes [`RecallBundle::context_text`] ever returns.
pub const MAX_CONTEXT_BYTES: usize = 8 * 1024;
/// Characters of any one line in [`RecallBundle::context_text`].
const MAX_LINE_CHARS: usize = 200;

/// Who is asking. Trust, membership and worker allowance are deliberately
/// absent: none of them is a reason to read memory.
///
/// Today only trusted local code constructs a `Caller`. Once recall is
/// reachable through an authenticated runtime path, the `Caller` must be
/// derived from that authenticated operator / orchestrator / peer context —
/// never accepted as caller-supplied authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    /// The human operating this node: every local episode.
    Operator,
    /// This node's orchestrator acting for one room: that room only.
    Orchestrator { room: String },
    /// Another node. V0 has no peer recall: always refused.
    Peer { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecallError {
    /// The caller may not read what it asked for.
    Denied(String),
    /// The query is malformed.
    Invalid(String),
    /// The store could not be read, or the named task's journal is corrupt.
    Store(String),
}

impl std::fmt::Display for RecallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Denied(why) => write!(f, "denied: {why}"),
            Self::Invalid(why) => write!(f, "invalid query: {why}"),
            Self::Store(why) => write!(f, "store: {why}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecallScope {
    pub task: String,
    pub room: String,
    /// Absent until a task carries a project authoritatively.
    pub project: Option<String>,
    pub peers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StepOutcome {
    pub id: String,
    pub capability: String,
    pub worker: String,
    /// complete | failed | blocked | not_run
    pub outcome: String,
    pub reason: Option<String>,
    /// `report/<id>`: the worker's signed room report, by reference.
    pub report: Option<String>,
}

/// One recalled episode: references first, no report text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecallItem {
    pub episode_id: String,
    pub task_id: String,
    pub scope: RecallScope,
    /// complete | failed — as the journal ended it.
    pub status: String,
    pub failure_reason: Option<String>,
    pub terminal_at_ms: u64,
    pub objective: String,
    pub capabilities: Vec<String>,
    pub workers: Vec<String>,
    pub step_outcomes: Vec<StepOutcome>,
    /// `report/<id>` of the accepted final report; `None` unless complete.
    pub final_report_ref: Option<String>,
    /// `report/<id>` of every report the evaluation rests on, failed
    /// tasks' successful steps included.
    pub evidence_refs: Vec<String>,
    /// Journal, Agent Wire and room message refs that explain the episode.
    pub provenance_refs: Vec<String>,
    /// Why it matched: every filter applied, as `dimension=value`.
    pub matched: Vec<String>,
    /// Whether any field of this item (objective, reason, list) was cut to
    /// its bound. Says nothing about how many episodes matched.
    pub clipped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecallBundle {
    /// The query as applied, scope gate included.
    pub query: Vec<String>,
    /// What the caller may read: `node/local` or `room/<name>`.
    pub scope: String,
    pub limit: usize,
    pub order: &'static str,
    pub episodes: Vec<RecallItem>,
    /// Every item's journal ref, in order.
    pub provenance: Vec<String>,
    /// True only when more episodes matched than `limit` returned. Field
    /// clipping is [`RecallItem::clipped`]; the formatter's byte bound is
    /// reported in [`RecallBundle::context_text`] itself.
    pub truncated: bool,
    /// Journals that could not be replayed: their tasks are withheld, never
    /// answered from a derived file.
    pub withheld: usize,
}

const ORDER: &str = "newest terminal first; task id descending on a tie";

/// Recall episodes for `caller`. Reads journals and brings derived episode
/// files in line with them (as [`memory::recall`] does); writes nothing
/// else, reaches no network, and returns data only.
pub fn recall(caller: &Caller, home: &Path, query: &Query) -> Result<RecallBundle, RecallError> {
    let (scope, query) = gate(caller, query)?;
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    query.check().map_err(RecallError::Invalid)?;
    let (hits, store) = memory::select(home, &query).map_err(RecallError::Store)?;
    if let Some(task) = &query.task
        && let Some((_, why)) = store.errors.iter().find(|(t, _)| t == task)
    {
        return Err(RecallError::Store(format!("journal {task}: {why}")));
    }
    let more = hits.len() > limit;
    let episodes: Vec<RecallItem> = hits.iter().take(limit).map(|e| item(e, &query)).collect();
    Ok(RecallBundle {
        query: applied(&query, limit),
        scope,
        limit,
        order: ORDER,
        truncated: more,
        provenance: episodes
            .iter()
            .map(|i| i.provenance_refs[0].clone())
            .collect(),
        episodes,
        withheld: store.errors.len(),
    })
}

/// The scope gate: who may read what. The returned query is the one run.
fn gate(caller: &Caller, query: &Query) -> Result<(String, Query), RecallError> {
    match caller {
        Caller::Operator => Ok(("node/local".into(), query.clone())),
        Caller::Orchestrator { room } => {
            let own = format!("room/{room}");
            if query.room.as_ref().is_some_and(|r| r != room) {
                return Err(RecallError::Denied(format!(
                    "orchestrator of {own} may not read another room"
                )));
            }
            if query
                .scope
                .as_ref()
                .is_some_and(|s| s.starts_with("room/") && *s != own)
            {
                return Err(RecallError::Denied(format!(
                    "orchestrator of {own} may not read another room"
                )));
            }
            let mut q = query.clone();
            q.room = Some(room.clone());
            Ok((own, q))
        }
        Caller::Peer { id } => Err(RecallError::Denied(format!(
            "peer/{id}: no peer recall in V0; trust and membership grant no memory access"
        ))),
    }
}

fn applied(q: &Query, limit: usize) -> Vec<String> {
    let mut out = filters(q);
    out.push(format!("recent={limit}"));
    out
}

fn filters(q: &Query) -> Vec<String> {
    [
        ("task", &q.task),
        ("status", &q.status),
        ("room", &q.room),
        ("capability", &q.capability),
        ("worker", &q.worker),
        ("scope", &q.scope),
    ]
    .into_iter()
    .filter_map(|(k, v)| v.as_ref().map(|v| format!("{k}={v}")))
    .collect()
}

fn clip(s: &str, max: usize, clipped: &mut bool) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if flat.chars().count() <= max {
        return flat;
    }
    *clipped = true;
    let mut out: String = flat.chars().take(max - 1).collect();
    out.push('…');
    out
}

fn bounded<T: Clone>(v: &[T], max: usize, clipped: &mut bool) -> Vec<T> {
    *clipped |= v.len() > max;
    v.iter().take(max).cloned().collect()
}

fn item(e: &Episode, q: &Query) -> RecallItem {
    let mut clipped = false;
    let p = &e.provenance;
    let mut provenance = vec![format!(
        "journal/{}#{} of {}",
        p.journal, p.terminal_record, p.records
    )];
    for s in &e.steps {
        if let Some(seq) = s.complete_seq {
            provenance.push(format!("wire/{}#{seq} COMPLETE {}", p.wire_history, s.id));
        }
    }
    provenance.push(format!("message/{} human", p.human_message));
    provenance.push(format!("message/{} accepted", p.accepted_message));
    if let Some(a) = &p.announcement {
        provenance.push(format!("message/{a} announced"));
    }
    let evidence: Vec<String> = e.evidence.iter().map(|r| format!("report/{r}")).collect();
    let steps: Vec<StepOutcome> = e
        .steps
        .iter()
        .map(|s| StepOutcome {
            id: s.id.clone(),
            capability: s.capability.clone(),
            worker: s.worker.clone(),
            outcome: s.outcome.clone(),
            reason: s
                .reason
                .as_deref()
                .map(|r| clip(r, MAX_REASON_CHARS, &mut clipped)),
            report: s.report.as_ref().map(|r| format!("report/{r}")),
        })
        .collect();
    let mut matched = filters(q);
    if matched.is_empty() {
        matched.push("any".into());
    }
    RecallItem {
        episode_id: e.episode_id.clone(),
        task_id: e.task_id.clone(),
        scope: RecallScope {
            task: e.scope.task.clone(),
            room: e.scope.room.clone(),
            project: e.scope.project.clone(),
            peers: bounded(&e.scope.peers, MAX_LIST, &mut clipped),
        },
        status: e.status.clone(),
        failure_reason: e
            .failure_reason
            .as_deref()
            .map(|r| clip(r, MAX_REASON_CHARS, &mut clipped)),
        terminal_at_ms: e.terminal_at_ms,
        objective: clip(&e.objective, MAX_OBJECTIVE_CHARS, &mut clipped),
        capabilities: bounded(&e.capabilities, MAX_LIST, &mut clipped),
        workers: bounded(&e.workers, MAX_LIST, &mut clipped),
        step_outcomes: bounded(&steps, MAX_LIST, &mut clipped),
        final_report_ref: e
            .final_report
            .as_ref()
            .filter(|_| e.status == "complete")
            .map(|r| format!("report/{r}")),
        evidence_refs: bounded(&evidence, MAX_LIST, &mut clipped),
        provenance_refs: bounded(&provenance, MAX_PROVENANCE, &mut clipped),
        matched,
        clipped,
    }
}

impl RecallItem {
    fn lines(&self) -> Vec<String> {
        let mut out = vec![
            format!("MEMORY EPISODE {}", self.episode_id),
            format!("status:: {}", self.status),
        ];
        if let Some(why) = &self.failure_reason {
            out.push(format!("failure:: {why}"));
        }
        out.push(format!("matched:: {}", self.matched.join(" ")));
        out.push(format!("scope:: {} · {}", self.scope.task, self.scope.room));
        out.push(format!("objective:: {}", self.objective));
        out.push(format!("capabilities:: {}", self.capabilities.join(", ")));
        out.push(format!("workers:: {}", self.workers.join(", ")));
        for s in &self.step_outcomes {
            let mut line = format!(
                "step:: {} {} → {} · {}",
                s.id, s.capability, s.worker, s.outcome
            );
            if let Some(r) = &s.reason {
                line.push_str(&format!(" ({r})"));
            }
            if let Some(r) = &s.report {
                line.push_str(&format!(" · {r}"));
            }
            out.push(line);
        }
        if let Some(r) = &self.final_report_ref {
            out.push(format!("final_report:: {r}"));
        }
        out.extend(self.evidence_refs.iter().map(|r| format!("evidence:: {r}")));
        out.extend(
            self.provenance_refs
                .iter()
                .map(|r| format!("provenance:: {r}")),
        );
        out
    }
}

impl RecallBundle {
    /// A small, provenance-rich text block, at most [`MAX_CONTEXT_BYTES`].
    /// Whole episodes only: one that does not fit is counted, not cut.
    /// A formatter only — nothing in /task::wire calls it.
    pub fn context_text(&self) -> String {
        let line = |s: &str| clip(s, MAX_LINE_CHARS, &mut false) + "\n";
        let mut out = line("MEMORY RECALL · read-only · grants no authority");
        out += &line(&format!("query:: {}", self.query.join(" ")));
        out += &line(&format!("scope:: {}", self.scope));
        out += &line(&format!("order:: {}", self.order));
        if self.episodes.is_empty() {
            out += "no episodes\n";
        }
        let footer = 64;
        let mut omitted = 0;
        for item in &self.episodes {
            let block: String = item.lines().iter().map(|l| line(l)).collect();
            if omitted == 0 && out.len() + block.len() + footer <= MAX_CONTEXT_BYTES {
                out += &block;
            } else {
                omitted += 1;
            }
        }
        if omitted > 0 {
            out += &format!("… {omitted} more episodes omitted (bound)\n");
        }
        if self.withheld > 0 {
            out += &format!("withheld:: {} unreadable journals\n", self.withheld);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal;
    use crate::memory::tests::{SECRET, home, task_in, task_said};
    use deaddrop_room::Status;
    use std::path::PathBuf;

    const ROOM: &str = "d34ddr0p";
    const RESEARCH: &str = "research:agent:deaddrop";

    fn seeded(test: &str) -> PathBuf {
        let home = home(&format!("recall-{test}"));
        task_in(&home, ROOM, "T-00000a", 100, Status::Ok);
        task_in(&home, ROOM, "T-00000b", 200, Status::Failed);
        task_in(&home, ROOM, "T-00000c", 300, Status::Ok);
        task_in(&home, "b4ckr00m", "T-00000d", 400, Status::Ok);
        home
    }

    fn ids(home: &Path, caller: &Caller, q: Query) -> Vec<String> {
        recall(caller, home, &q)
            .unwrap()
            .episodes
            .into_iter()
            .map(|i| i.task_id)
            .collect()
    }

    fn files(home: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        for dir in [journal::dir(home), memory::dir(home)] {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                out.push((e.path(), std::fs::read(e.path()).unwrap()));
            }
        }
        out.sort();
        out
    }

    #[test]
    fn every_dimension_matches_exactly_and_explains_itself() {
        let home = seeded("dimensions");
        let op = Caller::Operator;
        let q = |f: fn(&mut Query)| {
            let mut q = Query::default();
            f(&mut q);
            q
        };
        assert_eq!(
            ids(&home, &op, Query::default()),
            ["T-00000d", "T-00000c", "T-00000b", "T-00000a"]
        );
        assert_eq!(
            ids(&home, &op, q(|q| q.task = Some("T-00000b".into()))),
            ["T-00000b"]
        );
        assert_eq!(
            ids(&home, &op, q(|q| q.room = Some(ROOM.into()))),
            ["T-00000c", "T-00000b", "T-00000a"]
        );
        assert!(ids(&home, &op, q(|q| q.room = Some("d34".into()))).is_empty());
        assert_eq!(
            ids(&home, &op, q(|q| q.status = Some("failed".into()))),
            ["T-00000b"]
        );
        assert_eq!(
            ids(&home, &op, q(|q| q.worker = Some(RESEARCH.into()))).len(),
            4
        );
        assert!(ids(&home, &op, q(|q| q.worker = Some("research".into()))).is_empty());
        assert_eq!(
            ids(
                &home,
                &op,
                q(|q| q.capability = Some("github.inspect".into()))
            )
            .len(),
            4
        );
        assert!(ids(&home, &op, q(|q| q.capability = Some("github".into()))).is_empty());
        // Scope: task, room and peer exact; no project is ever invented.
        assert_eq!(
            ids(&home, &op, q(|q| q.scope = Some("task/T-00000a".into()))),
            ["T-00000a"]
        );
        assert_eq!(
            ids(&home, &op, q(|q| q.scope = Some("room/b4ckr00m".into()))),
            ["T-00000d"]
        );
        assert_eq!(
            ids(
                &home,
                &op,
                q(|q| q.scope = Some(format!("peer/{RESEARCH}")))
            )
            .len(),
            4
        );
        assert!(ids(&home, &op, q(|q| q.scope = Some("peer/research".into()))).is_empty());
        assert!(ids(&home, &op, q(|q| q.scope = Some("project/deaddrop".into()))).is_empty());
        // Combined filters, and why each result matched.
        let combined = Query {
            room: Some(ROOM.into()),
            capability: Some("github.inspect".into()),
            status: Some("complete".into()),
            limit: Some(3),
            ..Query::default()
        };
        let b = recall(&op, &home, &combined).unwrap();
        assert_eq!(
            b.episodes
                .iter()
                .map(|i| &i.task_id[..])
                .collect::<Vec<_>>(),
            ["T-00000c", "T-00000a"]
        );
        assert_eq!(
            b.episodes[0].matched,
            [
                "status=complete",
                "room=d34ddr0p",
                "capability=github.inspect"
            ]
        );
        assert_eq!(
            b.query,
            [
                "status=complete",
                "room=d34ddr0p",
                "capability=github.inspect",
                "recent=3"
            ]
        );
        assert!(b.episodes.iter().all(|i| i.scope.project.is_none()));
        let invalid = Query {
            scope: Some("everything".into()),
            ..Query::default()
        };
        assert!(matches!(
            recall(&op, &home, &invalid),
            Err(RecallError::Invalid(_))
        ));
        // There is no score anywhere: only exact reasons.
        let json = serde_json::to_string(&b).unwrap();
        assert!(!json.contains("score") && !json.contains("relevance"));
    }

    #[test]
    fn order_is_newest_terminal_first_with_a_stable_tie_break() {
        let home = home("recall-order");
        // Created out of order, two sharing a terminal time.
        task_in(&home, ROOM, "T-0000b2", 500, Status::Ok);
        task_in(&home, ROOM, "T-0000a1", 100, Status::Ok);
        task_in(&home, ROOM, "T-0000c3", 500, Status::Ok);
        let want = ["T-0000c3", "T-0000b2", "T-0000a1"];
        for _ in 0..3 {
            assert_eq!(ids(&home, &Caller::Operator, Query::default()), want);
        }
        let b = recall(&Caller::Operator, &home, &Query::default()).unwrap();
        assert_eq!(b.episodes[0].terminal_at_ms, b.episodes[1].terminal_at_ms);
        assert_eq!(b.order, ORDER);
    }

    #[test]
    fn recall_is_bounded() {
        let home = home("recall-bounds");
        for n in 0..(MAX_LIMIT + 3) {
            task_in(
                &home,
                ROOM,
                &format!("T-{n:06x}"),
                100 * n as u64,
                Status::Ok,
            );
        }
        let op = Caller::Operator;
        let b = recall(&op, &home, &Query::default()).unwrap();
        assert_eq!((b.limit, b.episodes.len(), b.truncated), (5, 5, true));
        let all = Query {
            limit: Some(1000),
            ..Query::default()
        };
        let b = recall(&op, &home, &all).unwrap();
        assert_eq!((b.limit, b.episodes.len()), (MAX_LIMIT, MAX_LIMIT));
        assert_eq!(b.query.last().unwrap(), "recent=20");
        let text = b.context_text();
        assert!(text.len() <= MAX_CONTEXT_BYTES, "{}", text.len());
        assert!(text.contains("more episodes omitted"));
        assert!(text.lines().all(|l| l.chars().count() <= MAX_LINE_CHARS));
        // Long human words are clipped, never the refs that explain them.
        let mut e = memory::select(&home, &Query::default()).unwrap().0[0].clone();
        e.objective = "x".repeat(10_000);
        e.capabilities = (0..30).map(|i| format!("cap{i}")).collect();
        let i = item(&e, &Query::default());
        assert!(i.clipped);
        assert_eq!(i.objective.chars().count(), MAX_OBJECTIVE_CHARS);
        assert_eq!(i.capabilities.len(), MAX_LIST);
        assert!(i.provenance_refs[0].starts_with("journal/tasks/"));
    }

    #[test]
    fn a_failed_task_is_recalled_as_failed_with_its_evidence() {
        let home = seeded("failed");
        let q = Query {
            capability: Some("github.inspect".into()),
            task: Some("T-00000b".into()),
            ..Query::default()
        };
        let b = recall(&Caller::Operator, &home, &q).unwrap();
        let [i] = b.episodes.as_slice() else {
            panic!("{b:?}")
        };
        assert_eq!(i.status, "failed");
        assert_eq!(
            i.failure_reason.as_deref(),
            Some("research_release failed (research failed); synthesize blocked")
        );
        let outcomes: Vec<&str> = i.step_outcomes.iter().map(|s| &s.outcome[..]).collect();
        assert_eq!(outcomes, ["failed", "complete", "blocked"]);
        // GitHub's successful evidence is kept, by reference.
        let github = i.step_outcomes[1].report.clone().unwrap();
        assert!(i.evidence_refs.contains(&github));
        // Nothing presents the failed synthesis as an answer.
        assert_eq!(i.final_report_ref, None);
        let text = b.context_text();
        assert!(text.contains("status:: failed") && !text.contains("status:: complete"));
        assert!(!text.contains("final_report::"));
    }

    #[test]
    fn every_item_carries_provenance_and_no_report_text() {
        let home = seeded("provenance");
        let b = recall(&Caller::Operator, &home, &Query::default()).unwrap();
        assert_eq!(b.episodes.len(), 4);
        for i in &b.episodes {
            let journal = format!("journal/tasks/{}.jsonl#", i.task_id);
            assert!(i.provenance_refs[0].starts_with(&journal), "{i:?}");
            assert!(i.provenance_refs.iter().any(|r| r.ends_with(" human")));
            assert!(i.evidence_refs.iter().all(|r| r.starts_with("report/")));
            assert_eq!(i.episode_id, format!("episode/{}", i.task_id));
            assert_eq!(i.scope.task, format!("task/{}", i.task_id));
        }
        let complete = &b.episodes[0];
        assert!(
            complete
                .provenance_refs
                .iter()
                .any(|r| r.starts_with("wire/") && r.ends_with("COMPLETE synthesize"))
        );
        assert!(complete.final_report_ref.is_some());
        assert_eq!(b.provenance.len(), 4);
        let json = serde_json::to_string(&b).unwrap() + &b.context_text();
        assert!(!json.contains("remember that") && !json.contains("Rust 1.99.0"));
        assert!(!json.contains(SECRET));
    }

    #[test]
    fn membership_trust_and_allowance_grant_no_memory() {
        let home = seeded("access");
        // A trusted peer that is an allowed worker on every one of these
        // tasks still reads nothing: V0 has no peer recall at all.
        let peer = Caller::Peer {
            id: RESEARCH.into(),
        };
        let own = Query {
            scope: Some(format!("peer/{RESEARCH}")),
            ..Query::default()
        };
        assert!(matches!(
            recall(&peer, &home, &own),
            Err(RecallError::Denied(_))
        ));
        // The orchestrator reads its own room only.
        let orch = Caller::Orchestrator { room: ROOM.into() };
        assert_eq!(
            ids(&home, &orch, Query::default()),
            ["T-00000c", "T-00000b", "T-00000a"]
        );
        let b = recall(&orch, &home, &Query::default()).unwrap();
        assert_eq!(b.scope, "room/d34ddr0p");
        assert_eq!(b.query, ["room=d34ddr0p", "recent=5"]);
        let elsewhere = Query {
            room: Some("b4ckr00m".into()),
            ..Query::default()
        };
        assert!(matches!(
            recall(&orch, &home, &elsewhere),
            Err(RecallError::Denied(_))
        ));
        let elsewhere = Query {
            scope: Some("room/b4ckr00m".into()),
            ..Query::default()
        };
        assert!(matches!(
            recall(&orch, &home, &elsewhere),
            Err(RecallError::Denied(_))
        ));
        let other_task = Query {
            task: Some("T-00000d".into()),
            ..Query::default()
        };
        assert!(ids(&home, &orch, other_task).is_empty());
        // The operator crosses rooms on their own node.
        assert_eq!(ids(&home, &Caller::Operator, Query::default()).len(), 4);
    }

    #[test]
    fn recall_mutates_nothing_and_is_not_wired_into_tasks() {
        let home = seeded("readonly");
        let op = Caller::Operator;
        recall(&op, &home, &Query::default()).unwrap();
        let before = files(&home);
        for _ in 0..3 {
            let b = recall(&op, &home, &Query::default()).unwrap();
            let _ = b.context_text();
        }
        assert_eq!(files(&home), before, "journals and episodes untouched");
        // Nothing in the task path reads memory: planning is unchanged.
        for src in [
            include_str!("planner.rs"),
            include_str!("plan.rs"),
            include_str!("run.rs"),
            include_str!("registry.rs"),
        ] {
            assert!(!src.contains("recall") && !src.contains("memory::"));
        }
    }

    #[test]
    fn journal_truth_wins_over_derived_files() {
        let home = seeded("corruption");
        let op = Caller::Operator;
        recall(&op, &home, &Query::default()).unwrap();
        // Stale: a failed task's file claims it completed.
        let p = memory::path(&home, "T-00000b");
        let stale = std::fs::read_to_string(&p)
            .unwrap()
            .replace("\"status\": \"failed\"", "\"status\": \"complete\"");
        std::fs::write(&p, stale).unwrap();
        let complete = Query {
            status: Some("complete".into()),
            ..Query::default()
        };
        assert!(!ids(&home, &op, complete).contains(&"T-00000b".to_owned()));
        assert!(
            std::fs::read_to_string(&p)
                .unwrap()
                .contains("\"status\": \"failed\"")
        );
        // Corrupt: rebuilt.
        std::fs::write(memory::path(&home, "T-00000a"), "{ not json").unwrap();
        assert!(ids(&home, &op, Query::default()).contains(&"T-00000a".to_owned()));
        // Missing: rebuilt.
        std::fs::remove_file(memory::path(&home, "T-00000c")).unwrap();
        assert!(ids(&home, &op, Query::default()).contains(&"T-00000c".to_owned()));
        assert!(memory::path(&home, "T-00000c").exists());
        // A corrupt journal fails closed: its episode file is never served.
        let j = journal::path(&home, "T-00000c");
        let text = std::fs::read_to_string(&j)
            .unwrap()
            .replacen("\"n\":2", "\"n\":9", 1);
        std::fs::write(&j, text).unwrap();
        let b = recall(&op, &home, &Query::default()).unwrap();
        assert!(b.episodes.iter().all(|i| i.task_id != "T-00000c"));
        assert_eq!(b.withheld, 1);
        assert!(
            b.context_text()
                .contains("withheld:: 1 unreadable journals")
        );
        let named = Query {
            task: Some("T-00000c".into()),
            ..Query::default()
        };
        assert!(matches!(
            recall(&op, &home, &named),
            Err(RecallError::Store(_))
        ));
    }

    #[test]
    fn truncated_means_only_more_matches_than_the_limit() {
        let op = Caller::Operator;
        // 4 matches, limit 5, one objective clipped: not truncated.
        let h1 = home("recall-truncated-clip");
        for (n, id) in ["T-0000e1", "T-0000e2", "T-0000e3"].iter().enumerate() {
            task_in(&h1, ROOM, id, 100 * n as u64, Status::Ok);
        }
        let long = "y".repeat(MAX_OBJECTIVE_CHARS * 4);
        task_said(&h1, ROOM, "T-0000e4", 900, Status::Ok, &long);
        let b = recall(&op, &h1, &Query::default()).unwrap();
        assert_eq!((b.limit, b.episodes.len()), (5, 4));
        assert!(b.episodes[0].clipped, "the long objective is clipped");
        assert!(b.episodes[1..].iter().all(|i| !i.clipped));
        assert!(!b.truncated);
        // 6 matches, limit 5: truncated.
        let h2 = home("recall-truncated-more");
        for n in 0..6u64 {
            task_in(&h2, ROOM, &format!("T-0000f{n}"), 100 * n, Status::Ok);
        }
        let b = recall(&op, &h2, &Query::default()).unwrap();
        assert_eq!((b.episodes.len(), b.truncated), (5, true));
        // The formatter's byte bound leaves the bundle as it was.
        let h3 = home("recall-truncated-context");
        for n in 0..MAX_LIMIT as u64 {
            task_said(&h3, ROOM, &format!("T-0001{n:02x}"), n, Status::Ok, &long);
        }
        let all = Query {
            limit: Some(MAX_LIMIT),
            ..Query::default()
        };
        let b = recall(&op, &h3, &all).unwrap();
        assert_eq!(b.episodes.len(), MAX_LIMIT);
        assert!(!b.truncated, "exactly the limit matched");
        let before = b.clone();
        let text = b.context_text();
        assert!(text.len() <= MAX_CONTEXT_BYTES);
        assert!(text.contains("more episodes omitted"));
        assert_eq!(b, before);
        assert!(!b.truncated);
    }
}
