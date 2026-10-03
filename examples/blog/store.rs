//! The data half: rows in memory, and the errors writing them can produce.
//!
//! Neither JSON:API nor axum appears here, exactly as in `todo_get` — this
//! is the layer you would swap for a database. What is new is that the
//! writes have *rules*: an author must exist before a post can reference
//! one, and a slug is unique. Those are storage's invariants, so they are
//! enforced here and named as typed errors, which [`crate::blog`] then
//! classifies into the right status code.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// One author. The target of a to-one relationship.
#[derive(Clone)]
pub struct Author {
    pub id: String,
    pub name: String,
}

/// One post, holding the *ids* of what it relates to rather than the rows.
///
/// Ids rather than `Author`/`Vec<Tag>` because that is what JSON:API
/// linkage is: a `type`/`id` pair. Storing the row would mean deciding here
/// how deep to load, which is a query concern, not a shape concern.
#[derive(Clone)]
pub struct Post {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub body: String,
    pub published: bool,
    /// The to-one.
    ///
    /// Named `author`, not `author_id`, because `resource!` reads the field
    /// whose name it was given: `author -> one "author"` looks for `author`.
    ///
    /// A `String` rather than an `Option<String>`: `resource!`'s `-> one`
    /// writes linkage from the field directly, so every post this store
    /// holds has an author. The *input* side is still `Option<String>` —
    /// absent linkage is not a parse error — which makes "no author given"
    /// a rule this service enforces rather than a shape it serializes. See
    /// `Create for Post` in `blog`, which turns the absence into a `422`.
    pub author: String,
    /// The to-many, likewise named for the relationship.
    pub tags: Vec<String>,
}

impl Post {
    /// Assemble a row from an id and the fields a write supplied.
    fn from_fields(id: String, fields: PostFields) -> Self {
        Self {
            id,
            title: fields.title,
            slug: fields.slug,
            body: fields.body,
            published: fields.published,
            author: fields.author,
            tags: fields.tags,
        }
    }
}

/// The mutable half of a [`Post`] — everything a write supplies, with the
/// id left out because the store assigns that on create and the path
/// carries it on update.
///
/// A struct rather than six positional parameters: `insert_post` and
/// `replace_post` take the same set, and three consecutive `String`s in an
/// argument list are three chances to swap two of them silently.
pub struct PostFields {
    pub title: String,
    pub slug: String,
    pub body: String,
    pub published: bool,
    pub author: String,
    pub tags: Vec<String>,
}

/// What writing to this store can fail with.
///
/// A typed enum rather than a `String`, so `Storage::classify` can match on
/// it exhaustively: add a variant and the compiler asks how a client should
/// see it, instead of letting it fall through to an opaque `500`.
#[derive(Debug)]
pub enum StoreError {
    /// The client asked for cursor paging; this store does offset only.
    UnsupportedPaging,
    /// Linkage naming an author that is not there. The client's mistake.
    NoSuchAuthor(String),
    /// A body that carried no `author` linkage at all.
    ///
    /// Separate from `NoSuchAuthor` because the two point at different
    /// places: one blames the id, the other its absence.
    MissingAuthor,
    /// Two posts cannot share a slug. Also the client's mistake, but a
    /// `409` rather than a `422`: the body is well-formed, the *world* is
    /// what disagrees with it.
    DuplicateSlug(String),
    /// `PATCH`/`DELETE` on an id that is not there.
    NotFound(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedPaging => f.write_str("cursor paging is not supported"),
            Self::NoSuchAuthor(id) => write!(f, "no author `{id}`"),
            Self::MissingAuthor => f.write_str("a post needs an author"),
            Self::DuplicateSlug(s) => write!(f, "slug `{s}` is taken"),
            Self::NotFound(id) => write!(f, "no post `{id}`"),
        }
    }
}

/// The application state: rows, plus the validators and hooks the JSON:API
/// layer needs. Cheap to clone — every field is an `Arc` — because axum
/// clones the state per request.
#[derive(Clone)]
pub struct App {
    pub posts: Arc<Mutex<BTreeMap<String, Post>>>,
    pub authors: Arc<Mutex<BTreeMap<String, Author>>>,
    /// Handed out by `create`. A real store would let the database do this.
    pub next_id: Arc<Mutex<usize>>,
    pub posts_validator: Arc<ash_jsonapi::validation::DocumentValidator>,
    pub authors_validator: Arc<ash_jsonapi::validation::DocumentValidator>,
    pub hooks: ash_jsonapi::hook::Hooks,
}

impl App {
    /// Two authors and two posts, so the relationships have something to
    /// point at before the first request.
    pub fn seeded(
        posts_validator: ash_jsonapi::validation::DocumentValidator,
        authors_validator: ash_jsonapi::validation::DocumentValidator,
        hooks: ash_jsonapi::hook::Hooks,
    ) -> Self {
        let authors = BTreeMap::from([
            (
                "a_1".to_string(),
                Author {
                    id: "a_1".into(),
                    name: "Ada".into(),
                },
            ),
            (
                "a_2".to_string(),
                Author {
                    id: "a_2".into(),
                    name: "Grace".into(),
                },
            ),
        ]);

        let posts = BTreeMap::from([
            (
                "p_1".to_string(),
                Post {
                    id: "p_1".into(),
                    title: "Hello".into(),
                    slug: "hello".into(),
                    body: "The first post.".into(),
                    published: true,
                    author: "a_1".into(),
                    tags: vec!["t_intro".into()],
                },
            ),
            (
                "p_2".to_string(),
                Post {
                    id: "p_2".into(),
                    title: "Drafting".into(),
                    slug: "drafting".into(),
                    body: "Not finished.".into(),
                    published: false,
                    author: "a_2".into(),
                    tags: vec![],
                },
            ),
        ]);

        Self {
            posts: Arc::new(Mutex::new(posts)),
            authors: Arc::new(Mutex::new(authors)),
            next_id: Arc::new(Mutex::new(3)),
            posts_validator: Arc::new(posts_validator),
            authors_validator: Arc::new(authors_validator),
            hooks,
        }
    }

    pub fn author(&self, id: &str) -> Option<Author> {
        self.authors.lock().unwrap().get(id).cloned()
    }

    pub fn authors_page(&self, offset: usize, limit: usize) -> (Vec<Author>, usize) {
        let rows = self.authors.lock().unwrap();
        let total = rows.len();
        (
            rows.values().skip(offset).take(limit).cloned().collect(),
            total,
        )
    }

    pub fn post(&self, id: &str) -> Option<Post> {
        self.posts.lock().unwrap().get(id).cloned()
    }

    /// One page of posts, sorted. The sort key is pre-validated by
    /// `Storage::sortable`, so an unknown one never reaches here.
    pub fn posts_page(
        &self,
        sort: Option<(&str, bool)>,
        offset: usize,
        limit: usize,
    ) -> (Vec<Post>, usize) {
        let rows = self.posts.lock().unwrap();
        let total = rows.len();
        let mut all: Vec<Post> = rows.values().cloned().collect();

        if let Some((field, descending)) = sort {
            all.sort_by(|a, b| {
                let ord = match field {
                    "title" => a.title.cmp(&b.title),
                    "slug" => a.slug.cmp(&b.slug),
                    "published" => a.published.cmp(&b.published),
                    _ => a.id.cmp(&b.id),
                };
                if descending { ord.reverse() } else { ord }
            });
        }

        (all.into_iter().skip(offset).take(limit).collect(), total)
    }

    /// Insert, enforcing both invariants: the author must exist, and the
    /// slug must be free.
    pub fn insert_post(&self, fields: PostFields) -> Result<Post, StoreError> {
        self.check_author(&fields.author)?;
        self.check_slug(&fields.slug, None)?;

        let mut next = self.next_id.lock().unwrap();
        let id = format!("p_{next}");
        *next += 1;

        let post = Post::from_fields(id.clone(), fields);
        self.posts.lock().unwrap().insert(id, post.clone());
        Ok(post)
    }

    /// Replace the row at `id`, enforcing the same invariants. The slug
    /// check excludes the row being edited — a post keeping its own slug is
    /// not a conflict.
    pub fn replace_post(&self, id: &str, fields: PostFields) -> Result<Post, StoreError> {
        if !self.posts.lock().unwrap().contains_key(id) {
            return Err(StoreError::NotFound(id.to_string()));
        }
        self.check_author(&fields.author)?;
        self.check_slug(&fields.slug, Some(id))?;

        let post = Post::from_fields(id.to_string(), fields);
        self.posts
            .lock()
            .unwrap()
            .insert(id.to_string(), post.clone());
        Ok(post)
    }

    /// Remove a row. Deleting something absent is an error rather than a
    /// silent success, so the client learns its id was wrong.
    pub fn remove_post(&self, id: &str) -> Result<(), StoreError> {
        self.posts
            .lock()
            .unwrap()
            .remove(id)
            .map(|_| ())
            .ok_or_else(|| StoreError::NotFound(id.to_string()))
    }

    pub fn insert_author(&self, name: String) -> Author {
        let mut next = self.next_id.lock().unwrap();
        let id = format!("a_{next}");
        *next += 1;

        let author = Author {
            id: id.clone(),
            name,
        };
        self.authors.lock().unwrap().insert(id, author.clone());
        author
    }

    fn check_author(&self, author: &str) -> Result<(), StoreError> {
        if !self.authors.lock().unwrap().contains_key(author) {
            return Err(StoreError::NoSuchAuthor(author.to_string()));
        }
        Ok(())
    }

    fn check_slug(&self, slug: &str, except: Option<&str>) -> Result<(), StoreError> {
        let taken = self
            .posts
            .lock()
            .unwrap()
            .values()
            .any(|p| p.slug == slug && Some(p.id.as_str()) != except);

        if taken {
            return Err(StoreError::DuplicateSlug(slug.to_string()));
        }
        Ok(())
    }
}
