//! The web UI: status, actions, run history, and snapshots to browse, download or restore. Pages are
//! rendered on the server. An open page receives its `<main>` again through server-sent events when
//! what it shows changes.
//!
//! An admin session acts as the server's account: whoever can start a restore or read a backup can
//! read and overwrite anything. So every request needs a login: the session cookie that `/login`
//! sets, or the admin token as `Authorization: Bearer`, which a reverse proxy can add. This also
//! defeats DNS rebinding. The metrics token opens only `/metrics` and `/status`. Requests that
//! browsers mark as cross-site (`Sec-Fetch-Site`) cannot change anything (CSRF).

mod actions;
mod assets;
mod auth;
mod browse;
mod collections;
mod events;
mod forms;
mod free;
mod label;
mod metrics;
mod repo;
mod route;
mod runs;
#[cfg(test)]
mod tests;

use crate::i18n::Language;
use assets::{asset_url, theme_colors};
use auth::Access;
pub(super) use auth::{Tokens, login_tokens};
pub(super) use browse::Browsed;
use label::{Button, Heading, Hint, Label};
pub(super) use route::Route;

use std::{any::Any, convert::Infallible, sync::Arc};

use askama::Template;
use axum::{
    Json, Router,
    extract::{FromRequestParts, State},
    http::{HeaderName, HeaderValue, StatusCode, header, request::Parts},
    middleware,
    response::{Html, IntoResponse, Response},
    routing::{delete, get, post},
};
use jiff::Timestamp;

use super::{App, Changes, Notice};
use crate::{APP_NAME, engine::Last, fmt, log::Level, shape, sys};

/// Every page links to the source code, as the AGPL requires for network use.
const SOURCE: &str = env!("CARGO_PKG_REPOSITORY");
pub(super) const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Pages load only resources from this server, and cannot be framed.
const CSP: &str = "default-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";
/// Inlined in every page: an external sprite would draw the icons after the page.
const ICONS: &str = include_str!("../../assets/images/sprite.svg");

pub(super) fn router(app: Arc<App>) -> Router {
    let status = async |State(app): State<Arc<App>>| {
        blocking(move || {
            let report = shape::to_value(&app.engine().report()).map_err(|e| Error::Internal(e.to_string()))?;
            Ok(Json(report).into_response())
        })
        .await
    };
    let guard = |page| {
        middleware::from_fn_with_state(app.clone(), move |access, base, req, next| {
            auth::logged_in(access, base, page, req, next)
        })
    };
    let at = |route: Route<'_>| route.pattern();
    let api = Router::new().route(&at(Route::Metrics), get(metrics::metrics)).route(&at(Route::Status), get(status));
    let pages = Router::new()
        .route(&at(Route::Index), get(collections::index))
        .route(&at(Route::Events), get(events::events))
        .route(&at(Route::Collection("")), get(collections::collection_page))
        .route(&at(Route::Run("", 0)), get(collections::collection_page).delete(actions::cancel))
        .route(&at(Route::Step("", 0, 0)), delete(actions::cancel_step))
        .route(&at(Route::Repo("", "")), get(repo::repo_page))
        .route(&at(Route::Logout), post(auth::logout))
        .route(&at(Route::Runs), post(actions::run))
        .route(&at(Route::Refresh), post(actions::refresh))
        .route(&at(Route::Setup), get(forms::setup).post(actions::save_setup))
        .route(&at(Route::SetupFree), get(free::free))
        .route(&at(Route::ProfileFolder), post(actions::open_profile_folder))
        .route(&at(Route::CollectionRuns("")), post(actions::run).delete(actions::cancel))
        .route(&at(Route::NewRun("")), get(forms::form))
        .route(&at(Route::CollectionRefresh("")), post(actions::refresh))
        .route(&at(Route::CollectionFolder("")), post(actions::open_collection_folder))
        .route(&at(Route::Failure("", "")), delete(actions::dismiss))
        .route(&at(Route::RepoRuns("", "")), post(actions::run))
        .route(&at(Route::RepoRefresh("", "")), post(actions::refresh))
        .route(&at(Route::Snapshot("", "", "")), get(browse::browse))
        .route(&at(Route::Download("", "", "")), get(browse::download))
        .route(&at(Route::Diff("", "", "")), get(browse::diff))
        .route(&at(Route::Find("", "")), get(browse::find));
    let pages = pages
        .route(&at(Route::Notifications("")), post(actions::notify))
        .route(&at(Route::SetupNotifications), post(actions::test_notification));
    Router::new()
        // For uptime monitors.
        .route(&at(Route::Health), get(|| async { "ok\n" }))
        .route(&at(Route::Login), get(auth::login_page).post(auth::login))
        .route(&at(Route::Asset("")), get(assets::asset))
        .merge(api.route_layer(guard(false)))
        .merge(pages.route_layer(guard(true)))
        .fallback(async || Error::NotFound(Missing::Page))
        // A known path with the wrong method is not found either.
        .method_not_allowed_fallback(async || Error::NotFound(Missing::Page))
        .layer(middleware::from_fn_with_state(app.clone(), auth::screen))
        .layer(middleware::map_response(async |mut res: Response| {
            for (name, value) in [
                (header::CONTENT_SECURITY_POLICY, CSP),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                (header::REFERRER_POLICY, "same-origin"),
                (HeaderName::from_static("cross-origin-opener-policy"), "same-origin"),
                (HeaderName::from_static("cross-origin-resource-policy"), "same-origin"),
            ] {
                res.headers_mut().insert(name, HeaderValue::from_static(value));
            }
            res
        }))
        // Outermost, so that every page and error is in the visitor's language.
        .layer(middleware::from_fn(async |req: axum::extract::Request, next: middleware::Next| {
            let accepted = req.headers().get(header::ACCEPT_LANGUAGE).and_then(|v| v.to_str().ok());
            let language = Language::accepted(accepted.unwrap_or_default());
            let mut res = language.scope_task(next.run(req)).await;
            if let Ok(tag) = HeaderValue::from_str(&language.to_string()) {
                res.headers_mut().insert(header::CONTENT_LANGUAGE, tag);
            }
            res.headers_mut().append(header::VARY, HeaderValue::from_static("accept-language"));
            res
        }))
        .with_state(app)
}

enum Error {
    NotFound(Missing),
    /// A rejected form: the dialog stays open.
    Refused(Notice),
    Internal(String),
}

#[derive(crate::i18n::Fluent)]
enum Missing {
    Page,
    Asset,
    RunKind,
    Collection,
    Run,
    Repository,
    Snapshot,
    /// The engine's explanation.
    #[fluent(transparent)]
    Named(String),
}

type Result<T = Response, E = Error> = std::result::Result<T, E>;

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        match self {
            Self::NotFound(what) => (StatusCode::NOT_FOUND, format!("{what}\n")).into_response(),
            Self::Refused(why) => (StatusCode::UNPROCESSABLE_ENTITY, flash(&why)).into_response(),
            Self::Internal(why) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{why}\n")).into_response(),
        }
    }
}

impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Self {
        Self::Internal(format!("{e:#}"))
    }
}

impl From<super::Fault> for Error {
    fn from(e: super::Fault) -> Self {
        anyhow::Error::from(e).into()
    }
}

/// Runs `f` outside the async runtime, so that other requests are served meanwhile, in the request's language.
async fn blocking(f: impl FnOnce() -> Result + Send + 'static) -> Response {
    let language = Language::current();
    tokio::task::spawn_blocking(move || language.scope(f))
        .await
        .unwrap_or_else(|e| Err(Error::Internal(e.to_string())))
        .into_response()
}

/// The path where the reverse proxy serves the web UI (`X-Forwarded-Prefix`), which starts every link.
/// Templates read it as `base`.
#[derive(Clone, Default)]
struct Base(String);

impl<S: Sync> FromRequestParts<S> for Base {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Infallible> {
        let prefix = parts.headers.get("x-forwarded-prefix").and_then(|v| v.to_str().ok());
        Ok(Self::new(prefix.unwrap_or_default()))
    }
}

impl Base {
    /// Empty unless it is a plain path on this host, so that no link can lead elsewhere.
    fn new(prefix: &str) -> Self {
        let prefix = prefix.trim_end_matches('/');
        let valid = prefix.starts_with('/')
            && !prefix.starts_with("//")
            && prefix.bytes().all(|b| b.is_ascii_alphanumeric() || b"/-._~".contains(&b));
        Self(if valid { prefix.to_owned() } else { String::new() })
    }

    fn to(&self, route: Route<'_>) -> String {
        format!("{}{route}", self.0)
    }
}

fn render_with(t: &impl Template, base: &Base) -> askama::Result<String> {
    let base: &dyn Any = &base.0;
    t.render_with_values(&("base", base))
}

fn render_str(t: &impl Template, base: &Base) -> String {
    render_with(t, base).unwrap_or_else(|e| format!("<p>{e}</p>"))
}

fn render(t: &impl Template, base: &Base) -> Response {
    render_with(t, base)
        .map_or_else(|e| Error::Internal(e.to_string()).into_response(), |html| Html(html).into_response())
}

/// A collection, and one of its runs: the one its page shows (unset: the newest), or the one to stop
/// (unset: every run).
#[derive(serde::Deserialize)]
struct RunPath {
    collection: String,
    id: Option<i64>,
}

/// The query string of a page's event stream: an `events::View`.
struct LiveQuery {
    v: u64,
    h: u64,
    collection: Option<String>,
    repo: Option<String>,
    run: Option<i64>,
    log: Option<(i64, u64)>,
}

/// Read before the page is rendered, so that a change during rendering is sent.
fn live(app: &App, collection: Option<&str>, repo: Option<&str>, run: Option<i64>) -> LiveQuery {
    let (collection, repo) = (collection.map(Into::into), repo.map(Into::into));
    LiveQuery { v: app.pages.changes.borrow().count, h: 0, collection, repo, run, log: None }
}

/// A whole page. `live`: the events that update its `<main>`.
#[derive(Template)]
#[template(path = "layout.html")]
struct Layout {
    title: String,
    live: Option<LiveQuery>,
    /// Logged in with a session, which can log out.
    logout: bool,
    main: String,
}

fn layout(title: &str, live: Option<LiveQuery>, access: Access, base: &Base, main: String) -> Response {
    let title = format!("{title} | {APP_NAME}");
    let live = live.map(|l| LiveQuery { h: sys::fnv1a(main.as_bytes()), ..l });
    render(&Layout { title, live, logout: access == Access::Session, main }, base)
}

/// The response to a request: a message in the page's flash area. The page itself is updated by its
/// event stream.
fn flash(notice: &Notice) -> Response {
    let html = render_str(&Flash::of(notice), &Base::default());
    ([("hx-retarget", "#flash"), ("hx-reswap", "innerHTML")], Html(html)).into_response()
}

#[derive(Template)]
#[template(path = "flash.html")]
struct Flash {
    level: &'static str,
    message: String,
}

impl Flash {
    fn of(notice: &Notice) -> Self {
        Self { level: notice.level().into(), message: notice.to_string() }
    }
}

#[derive(Template)]
#[template(path = "404.html")]
struct NotFound {
    /// What was not found, if the handler said.
    what: String,
}

/// The time pages are rendered at. Open pages are rendered again when what they show changes with it, so that
/// their ages stay current.
#[expect(clippy::disallowed_methods, reason = "pages show ages as of now")]
fn now() -> Timestamp {
    Timestamp::now()
}

/// A time shown as an age, or a word.
#[derive(Clone, Copy, PartialEq)]
enum Age {
    /// "2 hours ago", or a word.
    Last(Last),
    /// "2 hours": how long something has lasted.
    For(Timestamp),
    /// Not started yet.
    Queued,
}

impl std::fmt::Display for Age {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (t, age) = match *self {
            Age::Last(Last::At(t)) => (t, fmt::human_ago(t, now())),
            Age::For(t) => (t, fmt::human_age(t, now())),
            Age::Last(Last::Never) => return f.write_str(&Last::never()),
            Age::Last(Last::Unknown) => return write!(f, "{}", Label::Unknown),
            Age::Last(Last::Failing(_)) => return f.write_str(&Level::Failure.word()),
            Age::Queued => return Ok(()),
        };
        write!(f, r#"<time datetime="{t}" title="{}">{age}</time>"#, fmt::local(t))
    }
}

impl askama::filters::HtmlSafe for Age {}

/// A size in bytes, which `app.js` sorts by its `<data value>`. `None`: unknown.
#[derive(Clone, Copy)]
struct Size(Option<u64>);

impl std::fmt::Display for Size {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(bytes) => write!(f, r#"<data value="{bytes}">{}</data>"#, fmt::human_size(bytes)),
            None => write!(f, "{}", Label::Unknown),
        }
    }
}

impl askama::filters::HtmlSafe for Size {}
