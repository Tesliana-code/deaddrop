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

## Not yet

Nothing in `/task::wire` calls recall: the planner, Research, GitHub and
Klodik receive exactly what they did before. `RecallBundle::context_text()`
is a formatter only.

No `memory_read` wire event exists, because no runtime caller reads memory.
When one does, each read is to appear on the wire as:

```text
λ wire
memory_read:: episode/T-…
query:: capability=github.inspect
scope:: room/d34ddr0p
```
