//! What every page and response shares: rendering under the proxy's path, the layout around a page, a flash
//! message, the time pages show, and work done outside the async runtime.

use std::{any::Any, convert::Infallible};

use askama::Template;
use axum::{
    extract::FromRequestParts,
    http::request::Parts,
    response::{Html, IntoResponse, Response},
};
use jiff::Timestamp;

use super::{
    Access, App, Button, Error, Hint, ICONS, Label, Result, Route, SOURCE, VERSION,
    assets::{asset_url, theme_colors},
};
use crate::{APP_NAME, i18n::Language, server::Notice, sys};

/// Runs `f` outside the async runtime, so that other requests are served meanwhile, in the request's language.
pub(super) async fn blocking(f: impl FnOnce() -> Result + Send + 'static) -> Response {
    let language = Language::current();
    tokio::task::spawn_blocking(move || language.scope(f))
        .await
        .unwrap_or_else(|e| Err(Error::Internal(e.to_string())))
        .into_response()
}

/// The path where the reverse proxy serves the web UI (`X-Forwarded-Prefix`), which starts every link.
/// Templates read it as `base`.
#[derive(Clone, Default)]
pub(super) struct Base(pub(super) String);

impl<S: Sync> FromRequestParts<S> for Base {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Infallible> {
        let prefix = parts.headers.get("x-forwarded-prefix").and_then(|v| v.to_str().ok());
        Ok(Self::new(prefix.unwrap_or_default()))
    }
}

impl Base {
    /// Empty unless it is a plain path on this host, so that no link can lead elsewhere.
    pub(super) fn new(prefix: &str) -> Self {
        let prefix = prefix.trim_end_matches('/');
        let valid = prefix.starts_with('/')
            && !prefix.starts_with("//")
            && prefix.bytes().all(|b| b.is_ascii_alphanumeric() || b"/-._~".contains(&b));
        Self(if valid { prefix.to_owned() } else { String::new() })
    }

    pub(super) fn to(&self, route: Route<'_>) -> String {
        format!("{}{route}", self.0)
    }
}

pub(super) fn render_with(t: &impl Template, base: &Base) -> askama::Result<String> {
    let base: &dyn Any = &base.0;
    t.render_with_values(&("base", base))
}

pub(super) fn render_str(t: &impl Template, base: &Base) -> String {
    render_with(t, base).unwrap_or_else(|e| format!("<p>{e}</p>"))
}

pub(super) fn render(t: &impl Template, base: &Base) -> Response {
    render_with(t, base)
        .map_or_else(|e| Error::Internal(e.to_string()).into_response(), |html| Html(html).into_response())
}

/// The query string of a page's event stream: an `events::View`.
pub(super) struct LiveQuery {
    pub(super) v: u64,
    pub(super) h: u64,
    pub(super) collection: Option<String>,
    pub(super) repo: Option<String>,
    pub(super) run: Option<i64>,
    pub(super) log: Option<(i64, u64)>,
}

/// Read before the page is rendered, so that a change during rendering is sent.
pub(super) fn live(app: &App, collection: Option<&str>, repo: Option<&str>, run: Option<i64>) -> LiveQuery {
    let (collection, repo) = (collection.map(Into::into), repo.map(Into::into));
    LiveQuery { v: app.pages.changes.borrow().count, h: 0, collection, repo, run, log: None }
}

/// A whole page. `live`: the events that update its `<main>`.
#[derive(Template)]
#[template(path = "layout.html")]
pub(super) struct Layout {
    pub(super) title: String,
    pub(super) live: Option<LiveQuery>,
    /// Logged in with a session, which can log out.
    pub(super) logout: bool,
    pub(super) main: String,
}

pub(super) fn layout(title: &str, live: Option<LiveQuery>, access: Access, base: &Base, main: String) -> Response {
    let title = format!("{title} | {APP_NAME}");
    let live = live.map(|l| LiveQuery { h: sys::fnv1a(main.as_bytes()), ..l });
    render(&Layout { title, live, logout: access == Access::Session, main }, base)
}

/// The response to a request: a message in the page's flash area. The page itself is updated by its
/// event stream.
pub(super) fn flash(notice: &Notice) -> Response {
    let html = render_str(&Flash::of(notice), &Base::default());
    ([("hx-retarget", "#flash"), ("hx-reswap", "innerHTML")], Html(html)).into_response()
}

#[derive(Template)]
#[template(path = "flash.html")]
pub(super) struct Flash {
    level: &'static str,
    message: String,
}

impl Flash {
    pub(super) fn of(notice: &Notice) -> Self {
        Self { level: notice.level().into(), message: notice.to_string() }
    }
}

/// The time pages are rendered at. Open pages are rendered again when what they show changes with it, so that
/// their ages stay current.
#[expect(clippy::disallowed_methods, reason = "pages show ages as of now")]
pub(super) fn now() -> Timestamp {
    Timestamp::now()
}
