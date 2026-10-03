# Security policy

ash-jsonapi parses untrusted request bodies, query strings, filenames and media
types, and renders responses from them. A defect here reaches every service
built on the crate, so security reports get priority over everything else in
the tracker.

## Reporting a vulnerability

**Do not open a public issue.** Use GitHub's private vulnerability reporting:

> [**Report a vulnerability**](https://github.com/ashforge-rs/ash-jsonapi/security/advisories/new)
> — Security → Advisories → Report a vulnerability

That opens a private thread with the maintainers and, if the report is
accepted, becomes the advisory and CVE request without the details ever being
public first.

A report is most useful with:

- the ash-jsonapi version or commit, and the feature set in play — `audit`,
  `validation`, `domain` and `streaming` gate substantially different code;
- the resource declaration involved (the `resource!` or `domain_resource!`
  block), not the whole service;
- a request that demonstrates the behaviour — method, path, headers and body;
- what you expected the service to do, and what it did.

Expect an acknowledgement within **3 working days** and an assessment within
**10**. If a fix is warranted we will agree a disclosure date with you; the
default is publication once a release carrying the fix is out.

## Supported versions

| Version | Supported |
| --- | --- |
| `main` | Yes |
| pre-v1 tags | No — upgrade to `main` |

ash-jsonapi is **pre-v1** and not yet on crates.io. There is no maintained
release branch, so fixes land on `main`. Once v1 ships this table gains a
supported release line.

## What counts as a vulnerability

In scope — anything that breaks a property the crate claims:

- **Validation being bypassable**: a request body reaching a `Storage` impl
  that the derived JSON Schema should have refused, or an attribute being
  coerced to a type the declaration did not permit.
- **Header or response injection** through client-controlled data — a filename
  or media type breaking out of `Content-Disposition`, or a CR/LF splitting one
  header into two. `tests/files_safety.rs` is the standing guard on this.
- **Upload guards being defeated**: a body exceeding the configured size cap
  being read into memory, or a media type passing an allowlist it does not
  match (including through trailing parameters or case).
- **Path traversal** through an id or filename reaching the filesystem in the
  `CONTENT` operation.
- **Leakage through errors or audit**: an internal error message, a
  storage-layer detail or a principal's data appearing in a JSON:API error
  document that should not carry it, or unredacted data reaching an audit sink.
- **Authorization outcomes being lost**: with the `domain` feature, a policy
  denial in the domain pipeline surfacing as anything other than a refusal.
- A malformed document, query string or JSONL line being able to **panic,
  hang or allocate unboundedly** in a handler.

Out of scope — these are design boundaries rather than defects:

- **Authentication and authorization are yours.** The crate carries a
  `Context` with a principal and runs your `before` hooks; it does not
  authenticate anyone or decide what they may do. A service that never checks
  is not a crate vulnerability. (With `domain`, enforcement is `ash-domain`'s.)
- **Registering the audit hook is yours.** `audit` is on by default, but where
  records go is a deployment decision; nothing being recorded because no sink
  was wired is configuration, not a bug.
- **Rate limiting and request-size limits above the handler.** The crate caps
  `Upload` bodies, not connection volume; put a proxy in front.
- **What your `Storage` impl does** with a validated input — SQL injection in
  your own query construction, for instance.
- Findings from a scanner with no demonstrated impact on the above.

## Supply chain

`cargo deny check` covers advisories, licences, bans and sources on every pull
request. See [`deny.toml`](deny.toml).
