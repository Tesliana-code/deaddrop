# Authorization

**Status:** Public architectural contract

## 1. Observation is not authority

An agent's ability to read or reason about information does not automatically grant permission to mutate external state.

Deaddrop separates:

```text
READ
observe

PREPARE
construct a possible action

PROPOSE
present a concrete intended action

AUTHORIZE
grant bounded permission

ACT
perform the mutation

VERIFY
confirm the resulting authoritative state
```

Implementations may combine low-risk steps, but the conceptual boundaries must remain visible.

## 2. Consequential actions

Actions that may require explicit human or pre-delegated authorization include:

- purchases
- publication
- destructive operations
- account changes
- privilege changes
- contractual or external commitments
- financial operations
- security-sensitive configuration

## 3. Capability grants

A capability is a bounded permission.

It should answer:

- who may act?
- on what resource?
- which action?
- under what constraints?
- for how long?
- with what confirmation requirement?

Broad ambient authority should be avoided.

## 4. Least privilege

An agent handling one task should not automatically inherit every credential or tool available on the machine.

Capabilities should be scoped to the task and context.

## 5. Delegation

A peer or agent may delegate work without delegating all of its own authority.

Delegated authority must be no broader than the grant that permits it.

## 6. Verification

After a consequential action, the system should verify resulting state against the actual source authority where practical.

A success message from an agent is not a substitute for source evidence.

## 7. Human legibility

The UI should make it possible to distinguish:

- what the agent observed
- what it inferred
- what it proposes
- what it has authority to do
- what it actually did
- what source evidence confirms the result

## 8. Fail closed

Ambiguous authority is not permission.

When the system cannot determine whether an action is authorized, it should defer or request clarification.

## 9. Trust, policy and authority are separate

```text
TRUST GIVES YOU A CHANNEL.
POLICY GIVES YOU PERMISSION TO ASK.
AUTHORITY GIVES YOU PERMISSION TO ACT.
THE THREE ARE NOT THE SAME THING.
```

A trusted peer key authenticates messages; it does not let the sender invoke a peer's runtime. Whether a request may be answered is local, inspectable policy owned by the receiving peer's operator. Neither one grants side effects.

Reserved for the room slice: `0xd34ddr0p::wire`, a read-only stream of real machine routing events. Not implemented yet.
