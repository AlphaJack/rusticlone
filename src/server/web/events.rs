//! Live pages: their `<main>` is sent again as a server-sent event when what it shows changes.

use crate::i18n::Language;
use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
    extract::{Query, State},
    http::{HeaderMap, HeaderName},
    response::{
        IntoResponse,
        sse::{self, Sse},
    },
};
use futures_util::{StreamExt, stream};
use serde::Deserialize;
use tokio::sync::{broadcast, watch};

use super::{
    App, Base, Changes, Flash, Notice,
    auth::Here,
    collections::{collection_main, index_main},
    render_str,
    repo::{RepoMain, repo_view},
    runs::log_tail,
};
use crate::sys;

/// The shortest interval between two renders of a page while changes keep coming.
const RENDER_EVERY: Duration = Duration::from_millis(500);
/// How often a page is rendered again without a change, as its ages change once a minute.
pub(super) const AGE_TICK: Duration = Duration::from_mins(1);

#[derive(Clone, Deserialize)]
pub(super) struct View {
    collection: Option<String>,
    repo: Option<String>,
    run: Option<i64>,
    /// The change count the page was rendered at.
    v: Option<u64>,
    /// The hash of its `<main>`, which is not sent again.
    h: Option<u64>,
    /// The run whose log the page shows, and the log offset its lines end at.
    log: Option<i64>,
    at: Option<u64>,
    /// [`super::auth::Here`], from the stream's request rather than its query.
    #[serde(skip)]
    here: bool,
}

/// The `<main>` of a live page, and the run whose log it shows with the log offset. `None` if what it
/// shows no longer exists. `shown`: the log the page already shows, which it keeps.
fn render_main(
    app: &Arc<App>,
    view: &View,
    shown: Option<(i64, u64)>,
    base: &Base,
) -> Option<(String, Option<(i64, u64)>)> {
    match (&view.collection, &view.repo) {
        (None, _) => Some((render_str(&index_main(app, view.here), base), None)),
        (Some(collection), None) => {
            collection_main(app, collection, view.run, shown).map(|m| (render_str(&m, base), m.log.shown()))
        }
        (Some(collection), Some(repo)) => {
            repo_view(app, collection, repo).map(|repo| (render_str(&RepoMain { repo }, base), None))
        }
    }
}

/// The page's `<main>`, the log it shows, and that log's lines added since `shown`, if any.
fn render_update(app: &Arc<App>, view: &View, shown: Option<(i64, u64)>, base: &Base) -> Option<Update> {
    let (html, log) = render_main(app, view, shown, base)?;
    let (Some((run, at)), Some(collection)) = (log, &view.collection) else { return Some((html, log, None)) };
    let (lines, at) = log_tail(&app.engine(), collection, run, Some(at));
    Some((html, Some((run, at)), (!lines.0.is_empty()).then(|| render_str(&lines, base))))
}

/// What [`render_update`] renders.
type Update = (String, Option<(i64, u64)>, Option<String>);

/// What an event stream knows of its page.
struct Stream {
    app: Arc<App>,
    view: View,
    base: Base,
    changes: watch::Receiver<Changes>,
    /// The change count it was rendered at.
    seen: u64,
    failures: broadcast::Receiver<Notice>,
    /// It shows the current state: wait for the next change.
    wait: bool,
    /// The hash of its `<main>`.
    last: Option<u64>,
    rendered: tokio::time::Instant,
    /// The run whose log it shows, and the log offset its lines end at.
    log: Option<(i64, u64)>,
    /// New lines of that log, sent after `<main>`.
    lines: Option<sse::Event>,
    /// The page's, which renders outlive the request that opened the stream.
    language: Language,
}

/// Sends the page's `<main>` again when what it shows changed, at most every [`RENDER_EVERY`], and
/// only if it differs. A page's log grows through `log` events that carry only the new lines. Hidden
/// pages disconnect (htmx) and get everything again when they reconnect.
pub(super) async fn events(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    base: Base,
    Here(here): Here,
    Query(view): Query<View>,
) -> impl IntoResponse {
    let view = View { here, ..view };
    // What a reconnecting page shows is unknown.
    let reconnected = headers.contains_key("last-event-id");
    let (mut changes, failures) = (app.pages.changes.subscribe(), app.pages.failures.subscribe());
    let seen = view.v.filter(|_| !reconnected);
    let stream = Stream {
        app,
        // Rendered with the current state: wait for the next change.
        wait: seen.is_some_and(|v| {
            let changes = changes.borrow_and_update();
            v <= changes.count && !changes.since(v, view.collection.as_deref())
        }),
        seen: seen.unwrap_or_default(),
        last: view.h,
        rendered: tokio::time::Instant::now(),
        log: view.log.zip(view.at).filter(|_| !reconnected),
        lines: None,
        language: Language::current(),
        view,
        base,
        changes,
        failures,
    };
    let stream = stream::unfold(stream, |mut s| async move {
        if let Some(lines) = s.lines.take() {
            return Some((Ok::<_, Infallible>(lines), s));
        }
        let mut stopping = s.app.stopping.clone();
        loop {
            if s.wait {
                tokio::select! {
                    changed = s.changes.changed() => {
                        changed.ok()?;
                        if !s.changes.borrow_and_update().since(s.seen, s.view.collection.as_deref()) {
                            continue;
                        }
                    }
                    failure = s.failures.recv() => match failure {
                        Ok(why) => {
                            let flash = s.language.scope(|| render_str(&Flash::of(&why), &Base::default()));
                            return Some((Ok(sse::Event::default().event("flash").data(flash)), s));
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => return None,
                    },
                    _ = stopping.wait_for(|s| *s) => return None,
                    () = tokio::time::sleep(AGE_TICK) => {}
                }
                // Render once for a burst of changes.
                tokio::time::sleep_until(s.rendered + RENDER_EVERY).await;
            }
            (s.wait, s.rendered) = (true, tokio::time::Instant::now());
            s.seen = s.changes.borrow_and_update().count;
            let version = s.seen.to_string();
            let (app, view, shown, base, language) = (s.app.clone(), s.view.clone(), s.log, s.base.clone(), s.language);
            let render = move || language.scope(|| render_update(&app, &view, shown, &base));
            let (html, log, lines) = tokio::task::spawn_blocking(render).await.ok()??;
            s.log = log;
            s.lines = lines.map(|lines| sse::Event::default().event("log").id(&version).data(lines));
            // Unchanged: nothing to send.
            let hash = sys::fnv1a(html.as_bytes());
            if s.last.replace(hash) != Some(hash) {
                return Some((Ok(sse::Event::default().id(version).data(html)), s));
            }
            if let Some(lines) = s.lines.take() {
                return Some((Ok(lines), s));
            }
        }
    });
    // Send at once: some browsers hand the stream to the page only after its first bytes.
    let stream = stream::once(async { Ok(sse::Event::default().comment("live")) }).chain(stream);
    // Otherwise nginx buffers the events.
    ([(HeaderName::from_static("x-accel-buffering"), "no")], Sse::new(stream).keep_alive(sse::KeepAlive::default()))
}
