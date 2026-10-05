# Wire Command Language V0

**Status:** V0 contract. Parser implemented (`crates/deaddrop-wire-command`); no runtime integration.

## Purpose

A small, deterministic control syntax inside d34ddr0p messages: the future control surface between human chat and Agent Wire orchestration. Chat text and control syntax are different things, and **a parser decides which is which — never a language model.**

```text
human text → deterministic parser → ordinary chat
                                   | WireCommand (intent)
                                   | invalid command (error, not sent as chat)
```

## Axioms

```text
TRUST GIVES YOU A CHANNEL.
POLICY GIVES YOU PERMISSION TO ASK.
AUTHORITY GIVES YOU PERMISSION TO ACT.
ORCHESTRATION DECIDES WHO SHOULD ACT, AND IN WHAT ORDER.
THE FOUR ARE NOT THE SAME THING.
```

A parsed command is **user intent**. It grants no trust, no policy permission and no authority. A syntactically valid command may still be refused:

```text
parsed intent → trust → policy → capability resolution → authority → orchestration
```

## Grammar

```text
message      = payload-lines? command-line blank-lines?
command-line = "/" name "::wire" ( SP arg )*
name         = "objective" | "task" | "inspect" | "status" | "trace" | "cancel"
arg          = "--" flag | "--" flag "=" value | id
```

- **Final-line activation.** Only the last non-empty line can be a command. A command-looking line anywhere else is chat, so quoted examples never execute.
- **Command-shaped.** The final line is control syntax when its first token starts with `/` and contains `::wire` (in any case). It must then be exact; otherwise it is an error, never chat. `/task:wire` and `task::wire` are not command-shaped, so they are chat.
- **Canonical only.** Names are lowercase and exact. No aliases, no fuzzy matching, no autocorrection. `/Task::wire` is an unknown command.
- **Arguments** are whitespace-separated. No shell quoting in V0.
- **Payload** is the text above the command line. Leading blank lines, trailing whitespace and the blank separator lines before the command are removed. Everything in between is kept byte for byte, including newlines (Ctrl+J), indentation and Unicode. A trailing `\r` is ignored.

## Commands

| command | payload | arguments | meaning |
|---|---|---|---|
| `/objective::wire` | required | `--review` `--verbose` | A goal. The orchestrator decides decomposition, capabilities, workers and order. |
| `/task::wire` | required | `--dry-run` `--recall` | A concrete task to route and schedule, not reinterpret. |
| `/inspect::wire` | required | `--require=<cap>[,<cap>…]` (required) | Read-only evidence gathering using exactly the named capabilities. |
| `/status::wire` | none | `[<id>]` | Execution state: the current one, or `<id>`. |
| `/trace::wire` | none | `[<id>]` | The observable machine trace. Observation only. |
| `/cancel::wire` | none | `<id>` (required) | Stop future work for `<id>`. **Cancel is not rollback.** |

`status`, `trace` and `cancel` take no text. Text above them is an error rather than being silently dropped.

## Flags

- `--review`: build the plan, then wait for human approval before any worker starts.
- `--verbose`: human-facing orchestration detail (decomposition, capabilities, workers, dependencies, refusals, evaluation summary). Not raw logs; those are `/trace::wire`.
- `--dry-run`: syntax, policy evaluation, capability resolution and planning only. **No worker invocation, no external side effect, no quest execution.**
- `--recall`: the orchestrator reads this room's 3 most recent episodes and shows them to the planner as advisory history. Opt-in, room-scoped, bounded; it grants no trust, policy or authority, takes no value and names no scope. See `MEMORY_RECALL.md`.
- `--require=a.b,c.d`: capability ids. Each is dot-separated segments of `a-z 0-9 _ -`. At least one is required; empty items and duplicates are rejected.

Each flag may appear once. **Duplicates are rejected, not merged.** Boolean flags take no value. A flag valid for a different command is rejected for this one.

## Examples

```text
Proveri da li novi IBM RAG pristup ima nešto što možemo direktno
primeniti na TriageAI, uporedi sa našim repo obrascima i napravi zaključak.

/objective::wire --review --verbose
```

```text
Check whether commit 154f992 changes the TUI.

/inspect::wire --require=github.inspect
```

```text
/cancel::wire obj-42
```

## Errors

Concise, CLI-style, prefixed `wire:`:

```text
wire: objective requires payload
wire: cancel requires <id>
wire: unknown flag '--banana'
wire: task does not accept '--review'
wire: duplicate flag '--review'
wire: inspect requires at least one capability
wire: malformed capability 'github..inspect'
wire: malformed capability list 'a.b,,c.d'
wire: unknown command '/Task::wire'
wire: malformed command '/task::wire--dry-run'
wire: status takes no text; send it as its own message
wire: /help::wire is reserved for V1
```

## AST

```rust
enum Parsed { Ordinary, Command(WireCommand), Invalid(WireError) }

enum WireCommand {
    Objective { payload: String, review: bool, verbose: bool },
    Task      { payload: String, dry_run: bool },
    Inspect   { payload: String, require: Vec<CapabilityId> },
    Status    { id: Option<WireExecutionId> },
    Trace     { id: Option<WireExecutionId> },
    Cancel    { id: WireExecutionId },
}
```

`WireCommand::side_effects()` gives the ceiling a command's own semantics allow:
- `Never` for `inspect`, `status`, `trace`, `cancel` and `task --dry-run`.
- `SubjectToAuthority` otherwise.

There is no "granted" value.

## Orchestration contract (future)

The orchestrator decides structure. Workers decide bounded execution.

- **Orchestrator owns:** decomposition, capability resolution, assignment, dependency order, evaluation, and **completion authority**.
- **Workers own:** bounded computation, evidence and reports. A worker never declares an objective complete.
- **A candidate worker** must be capable, trusted and reachable, permitted by policy, *and* hold the authority the action needs. Having capability X never means it may use X for anyone.
- **`--review` UX:** show `objective:: accepted`, the proposed plan (worker → capability), dependencies, then `execute? y / n`. Nothing starts before approval.

## Agent Wire mapping

Inspected read-only: `agent-wire-quest-orchestrator-v0` @ a5339d1.

| command | Agent Wire today | gap |
|---|---|---|
| objective | `QuestSpec { objective, constraints, acceptance_gates, assignee }`; dependency subquests | **No planner**: nothing decomposes an objective or chooses workers by capability |
| task | `Orchestrator::assign(QuestSpec)` | Assignee is chosen explicitly; no capability resolution; no dry-run planning mode |
| inspect | `CapabilityId`; `github-inspector` worker (`github.inspect`) | No capability resolver; nothing enforces "read-only quest" yet |
| status | `Orchestrator::quests()` → `QuestState` | In-process only; no query API or execution ids exposed to chat |
| trace | `Orchestrator::events()`, `causal_chain(seq)` | In-process only; no stream for `0xd34ddr0p::wire` |
| cancel | — | **No cancel event or state exists** |

Completion authority already matches the contract: `Authority::Orchestrator` only, and workers never hold it.

## Product dialect

```text
[↓] 0xd34ddr0p

enter:: drop
ctrl+j:: newline

/objective::wire   /task::wire   /inspect::wire
/status::wire      /trace::wire  /cancel::wire

0xd34ddr0p::wire      reserved: the read-only observable machine stream
/help::wire           reserved for V1
```

`/trace::wire` is the command; `0xd34ddr0p::wire` is where traces will be shown. Neither exists at runtime yet.
