//! Logins: tokens, sessions, and cross-site requests.

use std::{
    convert::Infallible,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use askama::Template;
use axum::{
    Form,
    extract::{ConnectInfo, FromRequestParts, Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header, request::Parts},
    middleware::Next,
    response::{AppendHeaders, IntoResponse, Redirect, Response},
};
use jiff::SignedDuration;
use serde::Deserialize;

use super::{App, Base, Button, Error, Heading, Hint, Layout, NotFound, Route, layout, render, render_str};
use crate::{
    server::Fault,
    settings,
    sys::{random_hex, sha256},
};

/// The largest "not found" text a handler returns, read into the 404 page.
const NOT_FOUND_BODY: usize = 4096;

/// The session cookie. Over HTTPS it is `__Host-` prefixed, so that browsers bind it to this host and
/// path.
const COOKIE: &str = crate::NAME;
const COOKIE_HTTPS: &str = concat!("__Host-", env!("CARGO_PKG_NAME"));
/// The generated admin token's file in the state directory.
const TOKEN_FILE: &str = "admin-token";
/// Its mode: readable by its owner only.
#[cfg(unix)]
const TOKEN_MODE: u32 = 0o600;
const SESSION: SignedDuration = SignedDuration::from_hours(365 * 24);

/// A request that a browser sent on behalf of another site. Scripts send no such headers.
fn cross_site(headers: &HeaderMap) -> bool {
    let value = |name| headers.get(name).and_then(|v| v.to_str().ok());
    match (value(HeaderName::from_static("sec-fetch-site")), value(header::ORIGIN), value(header::HOST)) {
        (Some(site), ..) => !matches!(site, "same-origin" | "none"),
        // Browsers from before 2023.
        (None, Some(origin), Some(host)) => origin.split_once("://").is_none_or(|(_, origin)| origin != host),
        (None, ..) => false,
    }
}

/// What a request may do as the server's account.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::server) enum Access {
    /// The admin token: a script, or a reverse proxy that adds it.
    Token,
    /// A session from the login page, which can log out.
    Session,
    /// The metrics token: `/metrics` and `/status` only.
    Metrics,
    /// Only the login page, assets and `/health`.
    Anonymous,
}

fn session_hash(id: &str) -> String {
    sha256(id)
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let pairs = headers.get_all(header::COOKIE).into_iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';'));
    pairs.into_iter().find_map(|pair| pair.trim().strip_prefix(name)?.strip_prefix('='))
}

/// A `Max-Age` of 0 removes the cookie.
fn set_cookie(name: &str, value: &str, max_age: i64) -> (HeaderName, String) {
    let secure = if name == COOKIE_HTTPS { "; Secure" } else { "" };
    (header::SET_COOKIE, format!("{name}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Strict{secure}"))
}

/// A browser on this computer, logged in from the login page, so that what the server opens on its desktop is
/// in front of its user: not a script or a reverse proxy, which use the token or forward the request.
#[derive(Clone, Copy)]
pub(in crate::server) struct Here(pub(in crate::server) bool);

impl FromRequestParts<Arc<App>> for Here {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, app: &Arc<App>) -> Result<Self, Infallible> {
        let session = Access::from_request_parts(parts, app).await? == Access::Session;
        let peer = parts.extensions.get::<ConnectInfo<std::net::SocketAddr>>();
        let forwarded = [header::FORWARDED, HeaderName::from_static(FORWARDED_FOR)];
        let forwarded = forwarded.iter().any(|h| parts.headers.contains_key(h));
        Ok(Self(cfg!(feature = "desktop") && session && peer.is_some_and(|p| p.0.ip().is_loopback()) && !forwarded))
    }
}

/// What reverse proxies add, besides the standard `Forwarded`.
const FORWARDED_FOR: &str = "x-forwarded-for";

/// Computed once per request.
impl FromRequestParts<Arc<App>> for Access {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, app: &Arc<App>) -> Result<Self, Infallible> {
        if let Some(access) = parts.extensions.get::<Self>() {
            return Ok(*access);
        }
        let bearer = parts.headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()?.strip_prefix("Bearer "));
        let session = || {
            let id = [COOKIE_HTTPS, COOKIE].into_iter().find_map(|name| cookie(&parts.headers, name))?;
            // Invalid once the admin token changes.
            app.engine().store.session(&session_hash(id)).filter(|token| *token == app.tokens.admin.hash())
        };
        let access = match bearer.and_then(|bearer| app.tokens.access(bearer)) {
            Some(access) => access,
            None if session().is_some() => Access::Session,
            None => Access::Anonymous,
        };
        parts.extensions.insert(access);
        Ok(access)
    }
}

/// Guards the routes behind a login: the pages, or `/metrics` and `/status` (`page` false). A browser
/// without a login is sent to the login page when it asked for a page.
pub(super) async fn logged_in(access: Access, base: Base, page: bool, req: Request, next: Next) -> Response {
    match access {
        Access::Token | Access::Session => return next.run(req).await,
        Access::Metrics if !page => return next.run(req).await,
        Access::Metrics => {
            return (
                StatusCode::FORBIDDEN,
                format!(
                    "{}\n",
                    Hint::MetricsOnly { metrics: &Route::Metrics.to_string(), status: &Route::Status.to_string() }
                ),
            )
                .into_response();
        }
        Access::Anonymous => {}
    }
    if req.headers().contains_key("hx-request") {
        (StatusCode::UNAUTHORIZED, [("hx-redirect", base.to(Route::Login))]).into_response()
    } else if page && req.method() == Method::GET {
        Redirect::to(&base.to(Route::Login)).into_response()
    } else {
        (StatusCode::UNAUTHORIZED, format!("{}\n", Hint::LogInFirst)).into_response()
    }
}

/// Applies to every request: cross-site requests cannot change anything, a missing page gets a 404
/// page, and responses are not cached unless they say so.
pub(super) async fn screen(access: Access, base: Base, req: Request, next: Next) -> Response {
    let headers = req.headers();
    if !matches!(*req.method(), Method::GET | Method::HEAD) && cross_site(headers) {
        return (StatusCode::FORBIDDEN, format!("{}\n", Hint::CrossSite)).into_response();
    }
    let html = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("text/html"));
    let page = req.method() == Method::GET && !headers.contains_key("hx-request") && html;
    let mut response = next.run(req).await;
    if page && response.status() == StatusCode::NOT_FOUND {
        let body = axum::body::to_bytes(std::mem::take(response.body_mut()), NOT_FOUND_BODY).await.unwrap_or_default();
        let what = String::from_utf8_lossy(&body).trim().to_owned();
        let main = render_str(&NotFound { what }, &base);
        response =
            (StatusCode::NOT_FOUND, layout(&Heading::NotFound.to_string(), None, access, &base, main)).into_response();
    }
    response.headers_mut().entry(header::CACHE_CONTROL).or_insert(HeaderValue::from_static("no-store"));
    response
}

/// A login token. It has no `Debug` or `Display`, so that it is never printed by accident. It is
/// compared in constant time, so that a guess reveals nothing about the token.
pub(in crate::server) struct Token(String);

/// The admin token, the file it is kept in unless it is [`settings::ADMIN_TOKEN`], and the metrics token
/// if set.
pub(in crate::server) struct Tokens {
    pub(in crate::server) admin: Token,
    pub(in crate::server) file: Option<PathBuf>,
    metrics: Option<Token>,
}

impl Tokens {
    fn access(&self, guess: &str) -> Option<Access> {
        if self.admin == *guess {
            Some(Access::Token)
        } else {
            self.metrics.as_ref().is_some_and(|metrics| metrics == guess).then_some(Access::Metrics)
        }
    }
}

impl Token {
    fn hash(&self) -> String {
        sha256(&self.0)
    }

    /// For the server to print once, at start.
    pub(in crate::server) fn reveal(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for Token {
    fn eq(&self, guess: &str) -> bool {
        let token = self.0.as_bytes();
        token.len() == guess.len() && token.iter().zip(guess.bytes()).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
    }
}

#[derive(Template)]
#[template(path = "login.html")]
struct Login<'a> {
    failed: bool,
    file: Option<&'a Path>,
}

fn login_form(app: &App, failed: bool, base: &Base) -> Response {
    let main = render_str(&Login { failed, file: app.tokens.file.as_deref() }, base);
    let title = format!("{} | {}", Button::LogIn, super::APP_NAME);
    render(&Layout { title, live: None, logout: false, main }, base)
}

pub(super) async fn login_page(State(app): State<Arc<App>>, access: Access, base: Base) -> Response {
    // With an empty fragment: the browser would keep the token of `serve --open` in the address bar.
    if access != Access::Anonymous {
        return Redirect::to(&format!("{}#", base.to(Route::Index))).into_response();
    }
    login_form(&app, false, &base)
}

#[derive(Deserialize)]
pub(super) struct LoginForm {
    token: String,
}

pub(super) async fn login(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    base: Base,
    Form(form): Form<LoginForm>,
) -> Response {
    if app.tokens.admin != *form.token.trim() {
        return (StatusCode::UNAUTHORIZED, login_form(&app, true, &base)).into_response();
    }
    let session = async {
        let id = random_hex().map_err(Fault::NoSession)?;
        let (store, hash) = (app.engine().store.clone(), session_hash(&id));
        let token = app.tokens.admin.hash();
        tokio::task::spawn_blocking(move || store.add_session(&hash, store.now() + SESSION, &token))
            .await
            .map_err(anyhow::Error::from)
            .flatten()
            .map_err(|e| Fault::SessionNotKept(e.into()))?;
        anyhow::Ok(id)
    };
    match session.await {
        Ok(id) => {
            // Set by the reverse proxy.
            let https = headers.get("x-forwarded-proto").is_some_and(|v| v == "https");
            let name = if https { COOKIE_HTTPS } else { COOKIE };
            ([set_cookie(name, &id, SESSION.as_secs())], Redirect::to(&base.to(Route::Index))).into_response()
        }
        Err(e) => Error::from(e).into_response(),
    }
}

pub(super) async fn logout(State(app): State<Arc<App>>, headers: HeaderMap, base: Base) -> Response {
    let mut removed = Vec::new();
    for name in [COOKIE_HTTPS, COOKIE] {
        if let Some(id) = cookie(&headers, name) {
            _ = app.engine().store.remove_session(&session_hash(id));
            removed.push(set_cookie(name, "", 0));
        }
    }
    (StatusCode::NO_CONTENT, AppendHeaders(removed), [("hx-redirect", base.to(Route::Login))]).into_response()
}

/// The web UI's login tokens. Unless it is [`settings::ADMIN_TOKEN`], the admin token is a random one
/// in `dir`, generated the first time, so that sessions outlive a restart.
pub(in crate::server) fn login_tokens(dir: &Path) -> Result<Tokens> {
    let metrics = settings::METRICS_TOKEN.get(&[]).map(Token);
    if let Some(admin) = settings::ADMIN_TOKEN.get(&[]).map(Token) {
        return Ok(Tokens { admin, file: None, metrics });
    }
    let path = dir.join(TOKEN_FILE);
    Ok(Tokens { admin: Token(saved_token(&path)?), file: Some(path), metrics })
}

/// The token in `path`, written there first if absent.
#[expect(clippy::disallowed_methods, reason = "keeps the generated token")]
fn saved_token(path: &Path) -> Result<String> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, TOKEN_MODE);
    match options.write(true).create_new(true).open(path) {
        Ok(mut file) => {
            let token = random_hex().map_err(Fault::NoToken)?;
            file.write_all(token.as_bytes())?;
            Ok(token)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(std::fs::read_to_string(path)?.trim().to_owned()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_and_cross_site_requests() {
        let token = Token("abc".into());
        assert!(token == *"abc" && token != *"abd" && token != *"ab");
        let tokens = Tokens { admin: Token("admin".into()), file: None, metrics: Some(Token("metrics".into())) };
        assert!(tokens.access("admin") == Some(Access::Token) && tokens.access("metrics") == Some(Access::Metrics));
        assert!(tokens.access("").is_none() && Tokens { metrics: None, ..tokens }.access("metrics").is_none());
        let headers = |pairs: &[(&'static str, &'static str)]| {
            pairs.iter().map(|(k, v)| (HeaderName::from_static(k), HeaderValue::from_static(v))).collect::<HeaderMap>()
        };
        assert!(cross_site(&headers(&[("sec-fetch-site", "cross-site")])));
        assert!(cross_site(&headers(&[("sec-fetch-site", "same-site")])));
        assert!(!cross_site(&headers(&[("sec-fetch-site", "same-origin")])));
        assert!(!cross_site(&headers(&[("sec-fetch-site", "none")])));
        assert!(cross_site(&headers(&[("origin", "https://evil.example"), ("host", "backup.example")])));
        assert!(!cross_site(&headers(&[("origin", "https://backup.example"), ("host", "backup.example")])));
        assert!(!cross_site(&headers(&[("host", "backup.example")])), "not a browser");
    }
}
