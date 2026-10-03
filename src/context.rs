//! The per-request `Context`.
//!
//! Everything a request carries travels in one value, passed down explicitly:
//! the correlation id, the authenticated principal, the tenant, and any
//! metadata bound along the way. The `ash-log` scope lives *inside* it — see
//! [`Context::scope`] — so callers never build one themselves.
//!
//! Explicit passing rather than a thread-local is deliberate. `ash_log::Scope`
//! is bound per-thread, so it does not survive an `.await` that moves the task
//! to another worker. A `Context` is owned data: it crosses await points and
//! thread boundaries intact, and [`Context::scope`] re-enters the ash-log
//! scope wherever the work actually lands.

use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;

use crate::error::JsonApiError;

/// Everything one request carries.
///
/// Cheap to clone: the fields sit behind an [`Arc`], so handing a `Context` to
/// a nested call or another task copies a pointer.
///
/// ```ignore
/// use ash_jsonapi::context::Context;
///
/// let ctx = Context::new("01H9K2QZ8V")
///     .with_principal("alice@example.com")
///     .with_tenant("acme");
///
/// assert_eq!(ctx.correlation_id(), "01H9K2QZ8V");
/// assert_eq!(ctx.tenant(), Some("acme"));
/// ```
#[derive(Debug, Clone)]
pub struct Context {
    inner: Arc<Fields>,
}

#[derive(Debug, Clone)]
struct Fields {
    correlation_id: String,
    principal: Option<String>,
    tenant: Option<String>,
    locale: Option<crate::i18n::Locale>,
    metadata: BTreeMap<String, serde_json::Value>,
    /// Typed values attached to this request — see [`Context::with`].
    extensions: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl Context {
    /// A context for one request, identified by its correlation id.
    ///
    /// The id is what ties a response back to the audit record, so it is
    /// required rather than optional: it comes from the request's
    /// `X-Request-Id` where the client sent one, and is generated otherwise.
    pub fn new(correlation_id: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(Fields {
                correlation_id: correlation_id.into(),
                principal: None,
                tenant: None,
                locale: None,
                metadata: BTreeMap::new(),
                extensions: HashMap::new(),
            }),
        }
    }

    /// Bind the authenticated principal.
    ///
    /// Absent until authentication has run — an unauthenticated request has a
    /// context, just not a principal.
    #[must_use]
    pub fn with_principal(self, principal: impl Into<String>) -> Self {
        self.map(|f| f.principal = Some(principal.into()))
    }

    /// Bind the tenant, extracted per the resource's `TenantStrategy`.
    #[must_use]
    pub fn with_tenant(self, tenant: impl Into<String>) -> Self {
        self.map(|f| f.tenant = Some(tenant.into()))
    }

    /// Bind the locale the client asked for, negotiated from
    /// `Accept-Language`.
    ///
    /// Set by [`layer()`](crate::layer()) on every request that sends the
    /// header. Nothing in this crate translates — see [`i18n`](crate::i18n) —
    /// so this records what was asked for, and is echoed on the response's
    /// top-level `meta.lang`.
    #[must_use]
    pub fn with_locale(self, locale: crate::i18n::Locale) -> Self {
        self.map(|f| f.locale = Some(locale))
    }

    /// The locale the client asked for, if it sent an `Accept-Language` this
    /// crate could parse.
    pub fn locale(&self) -> Option<&crate::i18n::Locale> {
        self.inner.locale.as_ref()
    }

    /// Bind one metadata field, carried into every event logged under this
    /// context.
    #[must_use]
    pub fn with_metadata(
        self,
        key: impl Into<String>,
        value: impl Into<serde_json::Value>,
    ) -> Self {
        self.map(|f| {
            f.metadata.insert(key.into(), value.into());
        })
    }

    /// The correlation id. Always present.
    pub fn correlation_id(&self) -> &str {
        &self.inner.correlation_id
    }

    /// The authenticated principal, if authentication has run.
    pub fn principal(&self) -> Option<&str> {
        self.inner.principal.as_deref()
    }

    /// The tenant, if one was extracted.
    pub fn tenant(&self) -> Option<&str> {
        self.inner.tenant.as_deref()
    }

    /// Attach a typed value to this request.
    ///
    /// For what your own middleware resolves — an authenticated actor, a
    /// tenant handle, a feature flag set — so a handler receives one
    /// `Context` rather than a widening tuple. One value per type; attaching
    /// a second of the same type replaces the first.
    ///
    /// A parsed request body does *not* travel this way: it is handed to
    /// [`Create::create`](crate::crud::Create::create) as an argument, since
    /// a create always has one and routing it through here would turn an
    /// infallible step into a fallible lookup.
    ///
    /// ```ignore
    /// // in middleware, after authenticating
    /// let ctx = ctx.with(actor);
    ///
    /// // anywhere downstream
    /// let actor: &Actor = ctx.extract()?;
    /// ```
    #[must_use]
    pub fn with<T: Any + Send + Sync + 'static>(self, value: T) -> Self {
        self.map(|fields| {
            fields.extensions.insert(TypeId::of::<T>(), Arc::new(value));
        })
    }

    /// The value of type `T` attached to this request, if any.
    pub fn get_ext<T: Any + Send + Sync + 'static>(&self) -> Option<&T> {
        self.inner
            .extensions
            .get(&TypeId::of::<T>())
            .and_then(|value| value.downcast_ref::<T>())
    }

    /// The value of type `T`, or a `500` naming what was missing.
    ///
    /// An absent extension means a handler ran without the middleware or
    /// extractor that should have attached it — a wiring bug, not a bad
    /// request, so the client is told nothing beyond the correlation id.
    pub fn extract<T: Any + Send + Sync + 'static>(&self) -> Result<&T, JsonApiError> {
        self.get_ext::<T>().ok_or_else(|| {
            eprintln!(
                "[{}] no `{}` in the request context",
                self.correlation_id(),
                std::any::type_name::<T>()
            );
            JsonApiError::internal(self)
        })
    }

    /// One metadata value.
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.inner.metadata.get(key)
    }

    /// Every metadata field bound on this context.
    pub fn metadata(&self) -> &BTreeMap<String, serde_json::Value> {
        &self.inner.metadata
    }

    /// Copy-on-write update; a `Context` already shared stays untouched.
    fn map(mut self, edit: impl FnOnce(&mut Fields)) -> Self {
        let fields = Arc::make_mut(&mut self.inner);
        edit(fields);
        self
    }
}

#[cfg(feature = "audit")]
#[cfg_attr(docsrs, doc(cfg(feature = "audit")))]
impl Context {
    /// Enter the `ash-log` scope for this context.
    ///
    /// Every log line and audit event emitted while the returned guard lives
    /// inherits the correlation id, the principal and the metadata — with no
    /// threading through call signatures, and no scope built by hand at the
    /// call site.
    ///
    /// Hold the guard across the work, not across an `.await` that might move
    /// threads: `ash_log::Scope` is thread-local, so re-enter on the far side
    /// of a spawn instead. The `Context` itself crosses freely.
    ///
    /// ```ignore
    /// # use ash_jsonapi::context::Context;
    /// let ctx = Context::new("01H9K2QZ8V").with_principal("alice@example.com");
    ///
    /// let _guard = ctx.scope();
    /// // ash_info!/ash_audit! here carry the id and the principal.
    /// ```
    pub fn scope(&self) -> ash_log::ScopeGuard {
        let mut scope = ash_log::Scope::new().correlation_id(self.correlation_id());

        if let Some(principal) = self.principal() {
            scope = scope.principal(principal);
        }
        // The tenant is a scope field like any other, so an audit event
        // records which tenant a decision was made for.
        if let Some(tenant) = self.tenant() {
            scope = scope.with("tenant", tenant);
        }
        for (key, value) in self.metadata() {
            scope = scope.with(key.clone(), value.clone());
        }

        scope.enter()
    }
}
