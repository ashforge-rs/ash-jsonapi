//! A resource that *is* a file: upload it, read its metadata, download it.
//!
//! The shape JSON:API services use for binaries. An attachment is two things
//! at two URLs:
//!
//! - `/attachments/{id}` — an ordinary JSON:API document: name, media type,
//!   size. This is what a client lists, links to and caches.
//! - `/attachments/{id}/content` — the bytes themselves, served as their own
//!   media type.
//!
//! They are separate because a JSON:API document has nowhere to put bytes
//! except base64 in an attribute, which inflates the transfer by a third and
//! cannot be ranged, streamed or served by a CDN.

use ash_jsonapi::crud::{Content, Create, Read, Resource, Storage};
use ash_jsonapi::document::ResourceObject;
use ash_jsonapi::extract::Upload;
use ash_jsonapi::response::Download;
use ash_jsonapi::{Context, JsonApiError};

use crate::store::{App, StoreError};

/// One stored file.
#[derive(Clone)]
pub struct Attachment {
    pub id: String,
    pub bytes: Vec<u8>,
    pub content_type: String,
}

/// What this endpoint accepts.
///
/// An allowlist rather than "anything": a route that stores `text/html` and
/// serves it back is a stored-XSS vector, and the cheapest place to stop that
/// is before the bytes are ever written.
const ACCEPTED: &[&str] = &["image/png", "image/jpeg", "application/pdf", "text/plain"];

/// The largest upload: 2 MiB, well under the crate's own default.
const MAX: usize = 2 * 1024 * 1024;

/// The upload body, with this resource's limits applied.
///
/// A newtype over [`Upload`] so the size cap and the allowlist live with the
/// resource rather than being re-decided per handler.
pub struct NewAttachment(pub Upload);

impl ash_jsonapi::extract::FromRequest<App> for NewAttachment {
    async fn from_request(
        request: axum::extract::Request,
        _state: &App,
        ctx: &Context,
    ) -> Result<Self, ash_jsonapi::response::Errors> {
        Ok(Self(Upload::accepting(request, ctx, MAX, ACCEPTED).await?))
    }
}

// Written by hand rather than with `resource!`: the create body is a raw
// upload, not a JSON:API document, which is exactly the case `resource!`
// says to write out yourself.
impl Resource<App> for Attachment {
    type Create = NewAttachment;
    type Update = NewAttachment;

    const NAME: &'static str = "attachment";
    const PATH: &'static str = "/attachments";

    /// The metadata document. Never the bytes — those are at `/content`,
    /// which this links to so a client does not have to build the URL.
    fn to_resource(&self, _state: &App, _ctx: &Context) -> Result<ResourceObject, JsonApiError> {
        Ok(ResourceObject::new("attachment", &self.id)
            .attr("content_type", self.content_type.clone())
            .attr("size", self.bytes.len() as u64)
            .with_links(ash_jsonapi::Links::this(format!(
                "/attachments/{}/content",
                self.id
            ))))
    }
}

impl Storage<Attachment> for App {
    type Error = StoreError;
}

impl Create<Attachment> for App {
    async fn create(&self, input: NewAttachment, _ctx: &Context) -> Result<Attachment, StoreError> {
        Ok(self.store_attachment(input.0.bytes, input.0.content_type))
    }
}

impl Read<Attachment> for App {
    async fn get(&self, id: &str) -> Result<Option<Attachment>, StoreError> {
        Ok(self.attachment(id))
    }
}

impl Content<Attachment> for App {
    /// The bytes. `None` is a `404`, exactly as a missing row would be.
    async fn content(&self, id: &str) -> Result<Option<Download>, StoreError> {
        let Some(file) = self.attachment(id) else {
            return Ok(None);
        };

        // The filename is ours here, but `Download` escapes whatever it is
        // given — a name that came from a client cannot break the header.
        Ok(Some(
            Download::new(file.bytes, file.content_type).filename(format!("{id}.bin")),
        ))
    }
}
