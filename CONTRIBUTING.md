# Contributing to Deaddrop

Contributions are welcome.

Deaddrop is public because its protocol, privacy model, trust boundaries, and interoperability should survive contact with people who did not design them.

## Start here

Before substantial work, read:

1. [README](README.md)
2. [Constitution](docs/CONSTITUTION.md)
3. [Architecture](docs/ARCHITECTURE.md)
4. the domain document relevant to your change

## Good contribution areas

- local-first storage
- native clients
- Agent Wire coordination semantics
- protocol design
- message interoperability
- cryptographic review
- artifact integrity
- provenance
- safe peer UX
- authorization
- offline behavior
- accessibility
- conformance tests
- threat modeling
- documentation

## Out of scope by default

Deaddrop does not optimize for:

- engagement
- retention
- virality
- follower growth
- popularity ranking
- behavioral advertising
- hidden telemetry
- dark patterns
- unsolicited bulk messaging
- attention capture

## Constitutional review

Every material PR should ask:

- does this expose information to a party that did not need it before?
- does this widen recipient scope?
- does this create new indexing or behavioral telemetry?
- does this give an intermediary unnecessary knowledge?
- does this confuse memory, coordination history, or source authority?
- does this make human authority less legible?
- does this create hidden centrality?
- does this weaken provenance or artifact integrity?

If yes, document the tradeoff clearly.

## Security-sensitive work

Changes to cryptography, identity, authorization, peer discovery, transport, or trust semantics require explicit security review.

Do not include private operational defense mechanisms in public PRs.

## Agent Wire team

Agent Wire work belongs naturally in this repository when it implements public coordination primitives that obey the Deaddrop Constitution.

Please use [docs/AGENT_WIRE.md](docs/AGENT_WIRE.md) as the boundary.

## License

The project license is not yet frozen. Until it is, avoid submitting substantial third-party code that would make later license selection ambiguous.
