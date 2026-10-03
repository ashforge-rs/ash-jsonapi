//! The JSON:API half: two related resources, and the full write surface.
//!
//! Where `todo_get` is read-only, this is the other half of the crate:
//! `POST`, `PATCH` and `DELETE`, a to-one and a to-many relationship, and
//! the two error shapes writes introduce — a `422` for a body that is wrong
//! in itself, and a `409` for one that is only wrong given what is already
//! stored.
//!
//! As in `todo_get`, nothing here mentions axum, a router or a port.

use ash_jsonapi::crud::{Create, Delete, List, Listing, Read, Storage, Update};
use ash_jsonapi::{Context, JsonApiError, Page, Query};

// The rows themselves live in `store`; this file gives them their wire
// format. `pub use` so `http` can name them from here, beside the routes.
use crate::store::{App, PostFields, StoreError};
pub use crate::store::{Author, Post};

/// Where each collection is mounted. Named here rather than in
/// [`crate::http`] because it is a fact about the resource: `resource!` puts
/// it in every `self` link.
pub const POSTS_PATH: &str = "/posts";
pub const AUTHORS_PATH: &str = "/authors";

// The related resource, declared first because `Post` links to it.
//
// No relationships of its own: an author does not carry its posts, since a
// to-many that grows without bound belongs behind its own paged URL rather
// than inline in every author document.
ash_jsonapi::resource! {
    Author as "author" at "/authors",
    state: App,
    schema: AuthorSchema,
    input: NewAuthor,
    id: id,
    attributes: { name: String },
}

// The main resource. `relationships:` is the line `todo_get` has no use for:
// target type and arity are declared once and reach the linkage, both link
// URLs, and the request schema.
//
// On the generated `NewPost`, `author -> one` becomes `Option<String>` —
// absent linkage is not an error, it is a draft — and `tags -> many` becomes
// `Vec<String>`.
ash_jsonapi::resource! {
    Post as "post" at "/posts",
    state: App,
    schema: PostSchema,
    input: NewPost,
    id: id,
    attributes: { title: String, slug: String, body: String, published: bool },
    relationships: { author -> one "author", tags -> many "tag" },
}

// Both resources parse bodies, so both need a validator registered. This is
// the `registry!` form that also carries hooks — the shape a service with
// validation *and* an audit trail needs.
//
// Omitting a resource routed for CREATE or UPDATE is a wiring bug and is
// reported as one, rather than silently skipping validation.
ash_jsonapi::registry! {
    App { posts_validator: Post, authors_validator: Author }
    hooks: hooks
}

impl Storage<Post> for App {
    type Error = StoreError;

    /// What `?sort=` may name. Declaring it means an unknown field is
    /// refused with a `400` before `list` runs.
    fn sortable(&self) -> Option<&[&str]> {
        Some(&["id", "title", "slug", "published"])
    }

    /// Which failures are the client's fault rather than the server's.
    ///
    /// This is where writes earn their keep. Without it, referencing a
    /// missing author is a `500` — telling the client to retry a request
    /// that can never succeed. The match is exhaustive, so a new variant in
    /// `StoreError` makes the compiler ask how a client should see it.
    fn classify(&self, err: &Self::Error, ctx: &Context) -> Option<JsonApiError> {
        match err {
            StoreError::UnsupportedPaging => Some(ash_jsonapi::invalid_parameter!(
                ctx,
                "this service pages by `page[offset]`/`page[limit]`",
                "page"
            )),

            // A `422` naming the exact member at fault. The pointer is what
            // lets a client highlight the offending field rather than
            // showing a banner.
            // `pointer:` rather than the `invalid_attribute!` shorthand:
            // that one takes an attribute *name* and expands it to
            // `/data/attributes/…`, and this member is a relationship.
            StoreError::NoSuchAuthor(id) => Some(ash_jsonapi::jsonapi_error!(
                ctx,
                ash_jsonapi::ErrorCode::InvalidAttribute,
                format!("no author `{id}`"),
                pointer: "/data/relationships/author/data/id"
            )),

            // A `409`, not a `422`: the body is well-formed and would have
            // been accepted a moment earlier. What it conflicts with is the
            // state of the world, which is the distinction the two codes
            // exist to draw.
            StoreError::DuplicateSlug(slug) => Some(ash_jsonapi::conflict!(
                ctx,
                format!("slug `{slug}` is taken")
            )),

            // The absence of linkage, pointed at the relationship object
            // rather than at an id that was never sent.
            StoreError::MissingAuthor => Some(ash_jsonapi::jsonapi_error!(
                ctx,
                ash_jsonapi::ErrorCode::InvalidAttribute,
                "a post needs an `author` relationship",
                pointer: "/data/relationships/author"
            )),

            StoreError::NotFound(id) => {
                Some(ash_jsonapi::not_found!(ctx, format!("no post `{id}`")))
            }
        }
    }
}

impl Read<Post> for App {
    /// `GET /posts/{id}`. `None` becomes a JSON:API `404`.
    async fn get(&self, id: &str) -> Result<Option<Post>, StoreError> {
        Ok(self.post(id))
    }
}

impl List<Post> for App {
    /// `GET /posts`. Paging and sorting arrive parsed and bounds-checked.
    async fn list(&self, query: &Query, _ctx: &Context) -> Result<Listing<Post>, StoreError> {
        let Page::Offset { offset, limit } = query.page() else {
            return Err(StoreError::UnsupportedPaging);
        };

        let sort = query.sort().first().map(|k| (k.field(), k.descending()));
        let (rows, total) = self.posts_page(sort, *offset, *limit);
        Ok(Listing::new(rows).total(total))
    }
}

impl Create<Post> for App {
    /// `POST /posts`. The body arrives parsed *and already validated* — it
    /// was checked against `Post`'s JSON Schema before this ran, so a
    /// missing `title` or a `published` that is a string never reaches here.
    ///
    /// What is left is what a schema cannot know: that the author exists and
    /// the slug is free. Those are storage's invariants, and `classify`
    /// above turns them into the right status.
    async fn create(&self, input: NewPost, _ctx: &Context) -> Result<Post, StoreError> {
        self.insert_post(PostFields {
            title: input.title,
            slug: input.slug,
            body: input.body,
            published: input.published,
            // A to-one arrives as `Option<String>`, because a body that
            // omits the linkage parses fine — whether it is *allowed* to is
            // this service's rule, not the parser's.
            author: input.author.ok_or(StoreError::MissingAuthor)?,
            tags: input.tags,
        })
    }
}

impl Update<Post> for App {
    /// `PATCH /posts/{id}`. The path's id is authoritative over anything the
    /// body carries, so a body claiming a different id cannot move a row.
    async fn update(&self, id: &str, input: NewPost, _ctx: &Context) -> Result<Post, StoreError> {
        self.replace_post(
            id,
            PostFields {
                title: input.title,
                slug: input.slug,
                body: input.body,
                published: input.published,
                author: input.author.ok_or(StoreError::MissingAuthor)?,
                tags: input.tags,
            },
        )
    }
}

impl Delete<Post> for App {
    /// `DELETE /posts/{id}` — a `204` on success.
    async fn destroy(&self, id: &str) -> Result<(), StoreError> {
        self.remove_post(id)
    }
}

// The author resource: read and create, and no update or delete anywhere.
// One trait per operation means the absence is the whole story — a `DELETE
// /authors/a_1` is a `405`, because nothing routed it and no impl exists.

impl Storage<Author> for App {
    type Error = StoreError;

    fn sortable(&self) -> Option<&[&str]> {
        Some(&["id", "name"])
    }

    fn classify(&self, err: &Self::Error, ctx: &Context) -> Option<JsonApiError> {
        match err {
            StoreError::UnsupportedPaging => Some(ash_jsonapi::invalid_parameter!(
                ctx,
                "this service pages by `page[offset]`/`page[limit]`",
                "page"
            )),
            _ => None,
        }
    }
}

impl Read<Author> for App {
    async fn get(&self, id: &str) -> Result<Option<Author>, StoreError> {
        Ok(self.author(id))
    }
}

impl List<Author> for App {
    async fn list(&self, query: &Query, _ctx: &Context) -> Result<Listing<Author>, StoreError> {
        let Page::Offset { offset, limit } = query.page() else {
            return Err(StoreError::UnsupportedPaging);
        };

        let (rows, total) = self.authors_page(*offset, *limit);
        Ok(Listing::new(rows).total(total))
    }
}

impl Create<Author> for App {
    async fn create(&self, input: NewAuthor, _ctx: &Context) -> Result<Author, StoreError> {
        Ok(self.insert_author(input.name))
    }
}
