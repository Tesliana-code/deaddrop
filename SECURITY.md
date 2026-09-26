# Security Policy

Deaddrop is early-stage software.

It should not yet be treated as production-secure.

## Public security model

The public repository intentionally documents:

- security goals
- threat classes
- trust assumptions
- protocol invariants
- identity semantics
- authorization boundaries
- cryptographic interfaces
- artifact integrity
- privacy guarantees and limitations

See [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md).

## Private operational defense

Exact deployed defensive mechanisms are intentionally not documented in this repository when disclosure would reduce their effectiveness.

Examples may include:

- detection heuristics
- thresholds
- quarantine triggers
- abuse signatures
- anomaly scoring
- defensive routing behavior
- decoy mechanisms
- incident-response playbooks
- live operational countermeasures

> **Open the protocol. Keep defensive operations need-to-know.**

This boundary is not a substitute for sound cryptographic or protocol design.

## Vulnerability reporting

Do not open a public issue for vulnerabilities involving:

- private-message disclosure
- secret or key exposure
- identity impersonation
- authorization bypass
- cross-peer data leakage
- artifact substitution
- remote code execution
- protocol downgrade
- scalable abuse of deployed infrastructure

A dedicated private reporting channel will be documented before production deployment.

Until then, do not publish exploit details against active deployments.

## No guarantees yet

Security claims in early architecture documents are design goals and invariants, not a certification.

Cryptographic and protocol mechanisms require independent review before production use.
