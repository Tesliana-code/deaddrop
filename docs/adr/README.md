# Architecture Decision Records

Architecture Decision Records (ADRs) preserve **why** an important choice was made.

The Constitution defines invariants.

Architecture documents define the current system shape.

ADRs record decisions that could reasonably have gone another way.

Examples:

- cryptographic signature suite
- key storage strategy
- local database choice
- canonical wire serialization
- relay protocol
- artifact-transfer mechanism
- identity rotation semantics

## Suggested format

```text
# ADR-NNN: Title

Status:
Date:

## Context

What problem requires a decision?

## Decision

What are we choosing?

## Alternatives considered

What credible alternatives were evaluated?

## Consequences

What becomes easier, harder, safer, or more constrained?

## Constitutional impact

Which Deaddrop invariants are relevant?
```

An ADR must not be used to quietly override the Constitution.

If a decision requires changing a constitutional invariant, that is a constitutional amendment and should be treated accordingly.
