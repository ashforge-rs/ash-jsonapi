//! Where a JSON:API response actually spends its time.
//!
//! The shape that matters is a **listing**: one request, many rows, each with
//! relationships. A single-resource `GET` is dominated by I/O, but a page of
//! a hundred rows runs the serializer a hundred times, and whatever it does
//! per row is multiplied by the page size.
//!
//! Two halves are measured separately, because they are fixed by different
//! code and would otherwise mask each other:
//!
//! - `objects` — records → `Vec<ResourceObject>`, which is `Serializer` alone.
//! - `document` — that vector → the JSON text actually written to the socket.
//! - `end_to_end` — both, which is what a `LIST` handler does.
use std::collections::BTreeMap;

use ash_jsonapi::{Document, NoRelationships, Rel, Schema, Serializer};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

/// A resource with relationships — the case that multiplies link building.
struct Todo;

impl Schema for Todo {
    type Relationship = Rel;

    fn name(&self) -> &str {
        "todo"
    }

    fn base_path(&self) -> &str {
        "/api/v1/todos"
    }

    fn relationships(&self) -> &[Rel] {
        const RELS: &[Rel] = &[
            Rel::to_one("owner", "user"),
            Rel::to_many("tags", "tag"),
            Rel::to_one("project", "project"),
        ];
        RELS
    }
}

/// A flat resource, to separate per-relationship cost from per-row cost.
struct Note;

impl Schema for Note {
    type Relationship = NoRelationships;

    fn name(&self) -> &str {
        "note"
    }
    fn base_path(&self) -> &str {
        "/api/v1/notes"
    }
    fn relationships(&self) -> &[NoRelationships] {
        &[]
    }
}

/// One row's worth of prepared data, as a data layer would hand it over.
fn row(n: usize) -> (String, BTreeMap<String, serde_json::Value>) {
    let attributes = BTreeMap::from([
        (
            "message".to_string(),
            serde_json::json!("buy milk, eggs, and a reasonably long string"),
        ),
        ("done".to_string(), serde_json::json!(false)),
        ("priority".to_string(), serde_json::json!(3)),
        (
            "created".to_string(),
            serde_json::json!("2026-02-17T12:00:00Z"),
        ),
    ]);

    (format!("t_{n}"), attributes)
}

fn related() -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([
        ("owner".to_string(), vec!["u_1".to_string()]),
        (
            "tags".to_string(),
            vec!["g_1".to_string(), "g_2".to_string(), "g_3".to_string()],
        ),
        ("project".to_string(), vec!["p_1".to_string()]),
    ])
}

/// Records → resource objects: the serializer on its own.
fn objects(c: &mut Criterion) {
    let mut group = c.benchmark_group("objects");

    for size in [1usize, 25, 100] {
        let rows: Vec<_> = (0..size).map(row).collect();
        let related = related();

        group.bench_with_input(
            BenchmarkId::new("with_relationships", size),
            &size,
            |b, _| {
                b.iter(|| {
                    let serializer = Serializer::new(&Todo);
                    let objects: Vec<_> = rows
                        .iter()
                        .map(|(id, attributes)| {
                            serializer.serialize(id, attributes.clone(), &related)
                        })
                        .collect();
                    std::hint::black_box(objects)
                });
            },
        );

        let empty = BTreeMap::new();
        group.bench_with_input(BenchmarkId::new("flat", size), &size, |b, _| {
            b.iter(|| {
                let serializer = Serializer::new(&Note);
                let objects: Vec<_> = rows
                    .iter()
                    .map(|(id, attributes)| serializer.serialize(id, attributes.clone(), &empty))
                    .collect();
                std::hint::black_box(objects)
            });
        });
    }

    group.finish();
}

/// Resource objects → the JSON text written to the socket.
fn document(c: &mut Criterion) {
    let mut group = c.benchmark_group("document");

    for size in [1usize, 25, 100] {
        let serializer = Serializer::new(&Todo);
        let related = related();
        let objects: Vec<_> = (0..size)
            .map(row)
            .map(|(id, attributes)| serializer.serialize(&id, attributes, &related))
            .collect();

        // The document is built once, outside the loop: cloning a page of
        // resource objects per iteration would measure the clone, not the
        // render, and is not something a handler does — it serializes the
        // objects it just built.
        let document = Document::collection(objects);

        group.bench_with_input(BenchmarkId::new("to_string", size), &size, |b, _| {
            b.iter(|| std::hint::black_box(serde_json::to_string(&document).unwrap()));
        });
    }

    group.finish();
}

/// What a `LIST` handler actually does: rows in, response text out.
fn end_to_end(c: &mut Criterion) {
    let mut group = c.benchmark_group("end_to_end");

    for size in [25usize, 100] {
        let rows: Vec<_> = (0..size).map(row).collect();
        let related = related();

        group.bench_with_input(BenchmarkId::new("list", size), &size, |b, _| {
            b.iter(|| {
                let serializer = Serializer::new(&Todo);
                let objects: Vec<_> = rows
                    .iter()
                    .map(|(id, attributes)| serializer.serialize(id, attributes.clone(), &related))
                    .collect();

                let document = Document::collection(objects);
                std::hint::black_box(serde_json::to_string(&document).unwrap())
            });
        });
    }

    group.finish();
}

/// The path `resource!` generates — what a standalone user actually runs.
///
/// [`objects`] measures `Serializer::serialize`, the *bulk* entry point for a
/// caller that already holds prepared maps. A standalone resource does not:
/// `resource!` writes `to_resource` in terms of the fluent builder —
/// `record(id).attr(…).rel(…).build()` — so that is the path a `GET` goes
/// through, and it is the one worth measuring before it is worth changing.
fn standalone(c: &mut Criterion) {
    /// A row, as a standalone user's struct would hold it.
    ///
    /// Named `Row` rather than `Todo` so it does not shadow the schema of
    /// that name above, which this still serializes against.
    struct Row {
        id: String,
        message: String,
        done: bool,
        priority: u32,
        created: String,
        owner: String,
        tags: Vec<String>,
        project: String,
    }

    fn todo(n: usize) -> Row {
        Row {
            id: format!("t_{n}"),
            message: "buy milk, eggs, and a reasonably long string".to_string(),
            done: false,
            priority: 3,
            created: "2026-02-17T12:00:00Z".to_string(),
            owner: "u_1".to_string(),
            tags: vec!["g_1".to_string(), "g_2".to_string(), "g_3".to_string()],
            project: "p_1".to_string(),
        }
    }

    /// Exactly what `resource!` expands to, written out so the benchmark
    /// measures the generated shape rather than the macro.
    fn to_resource(row: &Row) -> ash_jsonapi::ResourceObject {
        Serializer::new(&Todo)
            .record(&row.id.to_string())
            .attr("message", row.message.clone())
            .attr("done", row.done)
            .attr("priority", row.priority)
            .attr("created", row.created.clone())
            .rel("owner", row.owner.to_string())
            .rel_many("tags", row.tags.iter().map(ToString::to_string))
            .rel("project", row.project.to_string())
            .build()
    }

    let mut group = c.benchmark_group("standalone");

    for size in [1usize, 25, 100] {
        let rows: Vec<Row> = (0..size).map(todo).collect();

        group.bench_with_input(BenchmarkId::new("to_resource", size), &size, |b, _| {
            b.iter(|| {
                let objects: Vec<_> = rows.iter().map(to_resource).collect();
                std::hint::black_box(objects)
            });
        });

        // The whole of a `LIST`: rows → objects → the bytes on the socket.
        group.bench_with_input(BenchmarkId::new("list", size), &size, |b, _| {
            b.iter(|| {
                let objects: Vec<_> = rows.iter().map(to_resource).collect();
                let document = Document::collection(objects);
                std::hint::black_box(serde_json::to_string(&document).unwrap())
            });
        });
    }

    group.finish();
}

criterion_group!(benches, objects, document, end_to_end, standalone);
criterion_main!(benches);
