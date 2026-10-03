//! The data half: rows in memory, and the errors reading them can produce.
//!
//! Neither JSON:API nor axum appears here. This is the layer you would swap
//! for a database — and the point of keeping it separate is that swapping it
//! touches neither [`crate::todo`] nor [`crate::http`].

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::attachment::Attachment;
use crate::todo::{SortField, Todo};

/// What reading from this store can fail with.
///
/// A typed enum rather than a `String`, so
/// [`Storage::classify`](ash_jsonapi::crud::Storage::classify) can match on
/// it exhaustively: adding a variant makes the compiler ask how a client
/// should see it, instead of letting it fall through to an opaque `500`.
#[derive(Debug)]
pub enum StoreError {
    /// The client asked for cursor paging; this store does offset only.
    UnsupportedPaging,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedPaging => f.write_str("cursor paging is not supported"),
        }
    }
}

/// The service state.
///
/// `rows` is a `BTreeMap` so listing order is stable, which is what makes
/// paging meaningful without a database to sort for us.
///
/// No validator: validators check request bodies, and this service accepts
/// none — see the `Validators` impl in [`crate::todo`].
#[derive(Clone)]
pub struct App {
    rows: Arc<Mutex<BTreeMap<String, Todo>>>,
    /// Uploaded files, kept in memory beside the todos. A real service puts
    /// the bytes on disk or in object storage and keeps only the key here.
    files: Arc<Mutex<Vec<Attachment>>>,
}

impl App {
    /// A store with a few rows already in it.
    pub fn seeded() -> Self {
        let rows = ["buy milk", "write docs", "ship it"]
            .into_iter()
            .enumerate()
            .map(|(n, message)| {
                let id = format!("t_{}", n + 1);
                let todo = Todo {
                    id: id.clone(),
                    message: message.to_string(),
                    done: n == 0,
                };
                (id, todo)
            })
            .collect();

        Self {
            rows: Arc::new(Mutex::new(rows)),
            files: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Store an uploaded file, returning it with its assigned id.
    pub fn store_attachment(&self, bytes: Vec<u8>, content_type: String) -> Attachment {
        let mut files = self.files.lock().unwrap();
        let file = Attachment {
            id: format!("f_{}", files.len() + 1),
            bytes,
            content_type,
        };
        files.push(file.clone());
        file
    }

    /// One stored file by id.
    pub fn attachment(&self, id: &str) -> Option<Attachment> {
        self.files
            .lock()
            .unwrap()
            .iter()
            .find(|file| file.id == id)
            .cloned()
    }

    /// One row by id, or `None`.
    pub fn fetch(&self, id: &str) -> Option<Todo> {
        self.rows.lock().unwrap().get(id).cloned()
    }

    /// One page of rows, plus how many there are in total.
    ///
    /// Ordering and slicing happen here because they are storage's business;
    /// *which* field is sortable was decided in [`crate::todo`], which is
    /// what owns the resource's attributes.
    pub fn page(
        &self,
        sort: Option<(SortField, bool)>,
        offset: usize,
        limit: usize,
    ) -> (Vec<Todo>, usize) {
        let rows = self.rows.lock().unwrap();
        let mut todos: Vec<Todo> = rows.values().cloned().collect();

        if let Some((field, descending)) = sort {
            match field {
                SortField::Message => todos.sort_by(|a, b| a.message.cmp(&b.message)),
                SortField::Done => todos.sort_by_key(|a| a.done),
                SortField::Id => todos.sort_by(|a, b| a.id.cmp(&b.id)),
            }
            if descending {
                todos.reverse();
            }
        }

        let total = todos.len();
        (todos.into_iter().skip(offset).take(limit).collect(), total)
    }
}
