# Memory Recall V0

Recall is a bounded, explainable, read-only view of episodic task memory.

```text
L0  signed history      immutable evidence of what happened
L1  working memory      bounded, disposable task context
L2  episodic memory     durable structured records of finished work
L3  durable knowledge   out of scope
```

History records what happened. Memory records what is worth reusing. Memory
grants no authority, and replay never restores revoked authority.

**Recall may inform a decision. It never becomes the decision authority.**

## Flow

```text
explicit structured query
  → scope gate (who is asking)
  → episodes derived from the task journals (journal truth wins)
  → newest terminal first, task id descending on a tie
  → bounded RecallBundle of reference-first RecallItems
  → the caller may use it; the caller gains nothing
```

Entry point: `deaddrop_task::recall::recall(&Caller, home, &Query)`.
Callers never see journal paths or parse episode files. The CLI equivalent is
`deaddrop-memory --home <dir> [filters] --recall [--json]`.

## Matching

Structured and exact only: task, status, room, worker, capability, scope
(`task/…`, `room/…`, `peer/…`, `project/…`) and recent N. No embeddings, no
similarity, no fuzzy text, no model relevance, no invented tags. Each item
lists the filters it matched (`room=d34ddr0p`, `capability=github.inspect`);
there is no score.

## Bounds

| bound | value |
|---|---|
| default results | 5 |
| maximum results | 20 |
| objective | 240 characters |
| failure / step reason | 160 characters |
| capabilities, workers, peers, steps, evidence refs | 8 each |
| provenance refs | 12 (journal ref always first) |
| `context_text()` | 8 KiB, whole episodes only |

`RecallBundle.truncated` has one meaning: more episodes matched than the
limit returned. A field cut to its bound marks only `RecallItem.clipped`.
When `context_text()` leaves out whole episodes to stay within 8 KiB, it says
so in its own text (`… N more episodes omitted (bound)`); the bundle is
unchanged.

## Provenance

Every item carries its episode id, task id, journal ref
(`journal/tasks/T-….jsonl#<terminal record>`), Agent Wire COMPLETE refs,
the human / accepted / announcement message refs, and `report/<id>` evidence
refs. Worker report text is never copied; fetch the history by ref.

A failed task is recalled as `failed`, with its reason, its step outcomes and
the evidence its successful steps produced. It has no final report.

## Scope and access

A project scope exists in the model but is always absent in V0: no task
carries a project authoritatively, and none is inferred from cwd, repo,
branch or prose.

| caller | may read |
|---|---|
| local operator | every local episode, across rooms |
| local orchestrator | its own room only; another room is refused |
| peer | nothing: V0 has no peer-facing recall |

### The skupljen rule

```text
member of the skupljen  ≠  access to all memory
```

A trusted peer has a channel. An allowed worker may be asked. Neither reads
an episode, another peer's transcript or another room's state. Future peer
recall must pass an explicit scope and policy check of its own.

### Caller boundary

Today only trusted local code constructs a `Caller`. When recall is exposed
through an authenticated runtime path, the `Caller` must be derived from the
authenticated operator, orchestrator or peer context — never accepted as
caller-supplied authority. A request that says "I am the operator" is not
the operator.

## Corruption

The journal is the authority. A missing, stale or corrupt episode file is
rebuilt from its journal before it is read. A corrupt journal fails closed:
its task is withheld (counted in `withheld`), a stale episode file is never
served in its place, and a query naming that task is an error.

## `/task::wire --recall`

The one runtime reader of memory. The human opts in per task:

```text
Summarize the current architecture boundary between Deaddrop and Agent Wire.

/task::wire --recall
```

- **Recall is opt-in.** Plain `/task::wire` reads no memory: the planner's
  input is byte for byte what it was, and no `memory_read` appears.
- **Recall is room-scoped.** The caller is derived, never named:
  `Caller::Orchestrator { room: <the task's room> }`. The command accepts no
  room, scope, caller or project; another room's episodes are never shown,
  however recent.
- **Recall is bounded.** The 3 most recent episodes of the room, complete and
  failed alike, newest terminal first (`recall::for_task`). Nothing is matched
  against the task text: no keywords, no ranking, no embeddings. The planner
  is handed `RecallBundle::context_text()` (≤ 8 KiB, refs first, no report
  bodies) after the task, under a preamble saying it is history only. A
  failed episode is shown as failed, with its reason and its successful
  evidence refs.
- **Recall is advisory.** The planner may read it; the deterministic
  validator, current policy (`DEADDROP_TASK_MAY_ASK`), trust, room membership,
  the capability registry and the dispatch re-check decide exactly as they do
  without it.
- **Recall does not restore authority.** An episode in which a worker
  succeeded does not let a task ask that worker once policy no longer does.
- **Workers do not receive direct memory-store access.** Research, GitHub and
  Klodik get only their step requests; no memory text is added to them, and
  there is no memory capability, endpoint or file access for peers.

Each read appears on the wire, local to you, by reference:

```text
λ wire
memory_read:: 3 episodes
scope:: room/d34ddr0p
refs:: episode/T-…, episode/T-…, episode/T-…
```

An empty read is still a read (`memory_read:: 0 episodes`, no `refs`).

A read that cannot establish truth stops the task before planning: no
planner call, no worker, a visible `task:: not started · …` and the draft
left unsent. It never falls back to planning without memory. Any withheld
(unreplayable) journal counts: its task might be this room's newest episode.

Durability. An accepted memory-assisted task's `task_accepted` journal record
carries `"recall":{"scope":"room/…","episodes":["episode/T-…",…]}` — refs
only, never the recalled text — and replay puts `memory_read` back into its
trace. A plain task's record has no `recall` key. The human's room message is
re-sent as written, ending `/task::wire --recall`. A memory-assisted task that
is rejected, fails to plan or is a dry run never reaches the journal; its read
is shown locally only (λ wire note and trace), as every never-run task is.

Rollback boundary. Once a `task_accepted` record carrying `recall` has been
written, binaries predating Explicit Task Recall V0 may reject that journal:
journal records are `deny_unknown_fields`. This is intentional fail-closed
behavior. Rollback compatibility across this checkpoint is not promised.

`--dry-run --recall` (either order) reads and plans locally, then stops:
`recall:: enabled`, `episodes:: N`, nothing was run.

## Not yet

No peer-facing recall, no memory in worker requests, no relevance selection,
no L3 knowledge, no automatic recall.
