# ash-jsonapi

[![crates.io](https://img.shields.io/crates/v/ash-jsonapi.svg)](https://crates.io/crates/ash-jsonapi)
[![docs.rs](https://docs.rs/ash-jsonapi/badge.svg)](https://docs.rs/ash-jsonapi)
[![CI](https://github.com/ashforge-rs/ash-jsonapi/actions/workflows/ci.yml/badge.svg)](https://github.com/ashforge-rs/ash-jsonapi/actions/workflows/ci.yml)
[![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](#minimum-supported-rust-version)

Serve [JSON:API](https://jsonapi.org) from [axum](https://github.com/tokio-rs/axum).

> **Disclaimer:** this repository contains AI-generated code.

Describe a resource once, and get the wire format for free: the `type`/`id`
envelope, relationship linkage, `self`/`related` links, spec-shaped errors with
JSON Pointers, and request-body validation.

```rust
ash_jsonapi::resource! {
    Todo as "todo" at "/todos",
    state: App,
    schema: TodoSchema,
    input: NewTodo,
    id: id,
    attributes: { message: String, done: bool },
}

ash_jsonapi::api! {
    operations: [ash_jsonapi::LIST, ash_jsonapi::CREATE, ash_jsonapi::READ],
    resource: Todo,
    name: "todo",
    path: "/todos",
    state: App,
}
```

That is `GET|POST /todos` and `GET /todos/{id}`, serialized to spec, with
bodies checked against a JSON Schema derived from the declaration. `routes()`
is an ordinary `axum::Router<App>` — mount it wherever the rest of your service
lives.

> **Status: pre-v1.** The API is still moving; expect breaking changes between
> minor versions until 1.0.

## What you write

Two macros, and they are independent — either can be replaced by the traits it
stands for.

- **`resource!`** writes the wire format: the schema, the request-body type and
  its parsing, the serialization, and the JSON Schema bodies are validated
  against.
- **`api!`** writes the routes. Name the operations you want from `LIST`,
  `CREATE`, `READ`, `UPDATE`, `DELETE`, `CONTENT`, `STREAM`, `INGEST` — or
  `CRUD` for the first five. **Only the operations you route require their
  traits**, so a read-only service implements `Read` and nothing else.

Storage is one trait per operation (`Create`, `Read`, `List`, `Update`,
`Delete`), implemented against your own state. There is no ORM here and no
database opinion: the crate owns the wire format, you own the data.

One layer is not optional — `ash_jsonapi::layer()` attaches the per-request
`Context` carrying the correlation id and principal. Without it every request
is a 500.

## Features

| Feature | Default | What it adds |
|---|---|---|
| `http` | yes | The axum integration: `api!`, the response types |
| `validation` | yes | `DocumentValidator` — request bodies vs. a JSON Schema |
| `audit` | yes | `audit::Audit` — one recorded event per operation |
| `domain` | no | [`ash-domain`](#using-ash-domain) integration |
| `streaming` | no | JSONL in and out: the `STREAM` and `INGEST` operations |

That is the whole list. TLS, health, metrics, OpenTelemetry, OpenAPI and
idempotency are planned rather than shipped, and are deliberately *not*
declared as features until they gate real code — a feature that compiles and
does nothing is worse than one that is absent.

`audit` is on by default rather than opt-in: a REST service that does not
record its authorization decisions is a liability. Registering the hook is
still yours, because where the records go is a deployment decision this crate
should not make.

### Standalone

With `default-features = false` you get the document, error and serialization
types alone — no axum, no async runtime, and **no `ash-*` crate in the tree at
all**. Implement `Schema` and `RelationshipDef` over your own types and those
layers work unchanged.

This is exercised rather than merely claimed: the wire-type tests carry no
`required-features` and `make test-bare` runs the suite that way.

### Using `ash-domain`

Optional, and off by default. Enable `domain` and `domain_resource!` writes the
same traits from the schema the domain already holds — no attribute, type or
relationship restated. Actions run through the domain's own pipeline, so
policies and extensions apply and a denial reaches the client as a `403`.

The integration is a second way in, not the way in.

## Beyond CRUD

- **Files** — `Upload` in, `Download` out, and a `CONTENT` operation routing
  `/{id}/content`.
- **JSONL** (`streaming`) — one resource per line for collections too large to
  be a document. `STREAM` sends them, selected by `Accept`; `INGEST` reads
  them, answering one result line per input line so a partial batch is visible
  rather than guessed at.
- **Hooks** — `before`/`after` around every operation; what audit, and later
  metrics, are built on.
- **i18n** — translation keys on errors, and a negotiated `Locale`.

## Installation

```sh
cargo add ash-jsonapi
# with the ash-domain integration
cargo add ash-jsonapi --features domain
```

## Examples

```bash
cargo run --example todo_get   # reads, paging, sorting, JSONL streaming
cargo run --example blog       # writes, relationships, validation, hooks
```

Both are standalone — no `domain` feature — which is what they exist to
demonstrate.

## Development

```bash
make pre-commit   # fmt, lint, check, feature matrix, tests, bare tests, doc-tests
```

The feature matrix is the thing most likely to break silently, so `make
features` compiles the library under each feature set individually, and `make
test-bare` builds and runs the tests with no features at all.

## Minimum supported Rust version

**1.88**, and the edition is 2024. This is a floor, not a pin: raising it is a
minor-version change, and the `toolchains` CI job is what keeps the declared version
honest rather than aspirational.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). To report a security issue, see
[SECURITY.md](SECURITY.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
