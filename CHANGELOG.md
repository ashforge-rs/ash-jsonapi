# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

For ash-jsonapi, the public contract that versioning applies to is **the Rust
API** — the exported types and traits, the `resource!`, `domain_resource!` and
`api!` macros and the shape they expect, and the feature names — together with
**the HTTP surface the macros generate**: the routes, the JSON:API documents on
the wire, and the error bodies. A change to a generated route or a serialized
document is a breaking change even where no Rust signature moves, because it
reaches clients that never compile against this crate.

The minimum supported Rust version is a floor rather than a pin. Raising it is
a minor-version change and is called out here.

## [Unreleased]

## [0.1.0] — 2026-10-03

First release.

### Added

- **The wire types** — JSON:API documents, resource objects, relationship
  linkage, `self`/`related` links, and spec-shaped errors carrying
  `source.pointer`. These need no features and put no `ash-*` crate in the
  tree.
- **`resource!`** — one declaration writes the schema, the request-body type
  and its parsing, the serialization, and the JSON Schema that bodies are
  validated against.
- **`api!`** — the axum routes, per operation: `LIST`, `CREATE`, `READ`,
  `UPDATE`, `DELETE`, `CONTENT`, `STREAM`, `INGEST`, and `CRUD` as shorthand
  for the first five. Only the operations routed require their storage traits.
- **`crud`** — one trait per operation over your own state, so the crate owns
  the wire format and you own the data.
- **`Query`** — `?page[…]` and `?sort=` parsed and checked, with sortable
  fields declared rather than reflected.
- **`layer()`** — the per-request `Context` carrying the correlation id and
  principal.
- **Files** — `Upload` in, `Download` out, and the `CONTENT` operation routing
  `/{id}/content`, with traversal and size limits enforced.
- **Hooks** — `before`/`after` around every operation.
- **i18n** — translation keys on errors and a negotiated `Locale`.
- **`validation`** (default) — request bodies checked against the derived JSON
  Schema before anything reaches the data layer; each failure becomes a
  JSON:API error with a JSON Pointer.
- **`audit`** (default) — one recorded event per operation, and `Context::scope()`
  binding the correlation id and principal onto every line logged inside it.
- **`streaming`** — JSONL in and out: `STREAM` renders rows as they arrive
  rather than buffering, `INGEST` answers one result line per input line.
- **`domain`** — the `ash-domain` seam: `domain_resource!` derives the same
  traits from the schema the domain already holds, and actions run through the
  domain's pipeline so policy denials surface as `403`.
