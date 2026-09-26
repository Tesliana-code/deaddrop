# Artifacts

**Status:** Public architecture baseline

## 1. Why artifacts exist

Messages are good for coordination.

They are not always the right place for durable evidence, large outputs, datasets, patches, documents, or binary results.

Deaddrop therefore separates messages from artifacts.

## 2. Content identity

Where integrity matters, an artifact is referenced by the identity of its bytes.

Canonical form:

```text
sha256:<digest>
```

Example:

```text
sha256:9f2c…
```

A filename is a label.

A URL is a location.

A hash identifies content.

## 3. Immutable references

If bytes change, their content identity changes.

This allows a peer to verify that the artifact it retrieved is the artifact the sender referenced.

## 4. Metadata

Artifact metadata may include:

- media type
- byte length
- human label
- producer
- creation time
- source provenance
- relationship to a task or message

Metadata must not be allowed to replace content integrity.

## 5. Mutable aliases

Implementations may provide friendly mutable names or locations.

Those aliases must remain distinguishable from immutable references.

Example:

```text
"latest-report"
        ↓
sha256:abc...

later:

"latest-report"
        ↓
sha256:def...
```

Historical evidence should preserve the hash that was actually used.

## 6. Transport independence

An artifact may be obtained through:

- local storage
- direct peer transfer
- HTTP/object storage
- relay-assisted transfer
- future content-addressed peer transport

The reference should remain stable across transport changes.

## 7. Provenance

Artifact provenance should record enough context to understand:

- who or what produced it
- which source inputs it depended on
- which message/task referenced it
- whether it is original evidence, transformation, or generated output

## 8. Privacy

Content addressing does not make content public.

Knowing a hash must not be treated as consent to global indexing or publication.

## 9. Garbage collection

Local nodes may eventually remove unneeded artifact bytes.

Removal policy is local storage policy and must not rewrite historical references to pretend the artifact never existed.

## 10. Security

Artifact retrieval must verify expected content identity before trusted use.

A familiar filename, URL, sender label, or UI representation is not sufficient evidence of integrity.
