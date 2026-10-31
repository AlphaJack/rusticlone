//! Actions that pages start, cancel, dismiss or refresh, for every collection, one collection or one repository,
//! as the path names it.

use askama::Template;
use std::{path::PathBuf, sync::Arc};

use axum::{
    Form, RequestPartsExt,
    extract::{FromRequestParts, Path, RawPathParams, State},
    http::{HeaderName, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use super::{
    App, Base, Button, Error, Heading, Hint, Label, Missing, Notice, Result, Route, RunPath, auth::Here, blocking,
    flash, render_str,
};
use crate::{
    first_run,
    request::{RestoreOptions, RunKind, Selection},
    rustic::NotASnapshotId,
};

/// The selection that the path names, if it exists: `/runs`, `/collections/{collection}/runs`,
/// `/collections/{collection}/repos/{repo}/runs`.
impl FromRequestParts<Arc<App>> for Selection {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, app: &Arc<App>) -> Result<Self, Response> {
        let params = parts.extract::<RawPathParams>().await.ok();
        let param = |name: &str| Some(params.as_ref()?.iter().find(|(k, _)| *k == name)?.1.to_owned());
        let selection = match (param("collection"), param("repo")) {
            (Some(collection), Some(repo)) => Selection::Repo(collection, repo),
            (Some(collection), None) => Selection::Collection(collection),
            (None, _) => Selection::All,
        };
        match app.engine().named(&selection) {
            Ok(_) => Ok(selection),
            Err(e) => Err(Error::NotFound(Missing::Named(format!("{e:#}"))).into_response()),
        }
    }
}

/// The collection a notice concerns.
fn whose(app: &App, selection: &Selection) -> String {
    selection.collection().map_or_else(|| Label::EveryCollection.to_string(), |c| app.engine().title(c))
}

/// The installer's command on this system, without `install`: on a new computer, the server would back up the
/// folders before they are restored.
#[cfg(windows)]
const INSTALLER: &str = concat!(
    "irm ",
    env!("CARGO_PKG_REPOSITORY"),
    "/releases/latest/download/",
    env!("CARGO_PKG_NAME"),
    "-installer.ps1 | iex"
);
#[cfg(not(windows))]
const INSTALLER: &str = concat!(
    "curl -LsSf ",
    env!("CARGO_PKG_REPOSITORY"),
    "/releases/latest/download/",
    env!("CARGO_PKG_NAME"),
    "-installer.sh | sh"
);

/// Where the profiles were saved, and how to restore from them on a new computer.
#[derive(Template)]
#[template(path = "setup_saved.html")]
pub(super) struct SetupSaved {
    pub(super) dir: String,
    /// A browser on this computer, which can open their folder.
    pub(super) here: bool,
    /// The command lines that restore every collection, then back up as before.
    pub(super) restore: String,
    pub(super) resume: String,
}

/// Also the query of the dialog that asks for it, which fills in what it gives.
#[derive(Deserialize)]
pub(super) struct RunForm {
    pub(super) kind: RunKind,
    /// The repository a restore or test restore reads, or a re-create re-creates.
    #[serde(default)]
    pub(super) repo: String,
    #[serde(default)]
    pub(super) snapshot: String,
    #[serde(default)]
    pub(super) path: String,
    /// A restore that says what it would do.
    #[serde(default)]
    pub(super) dry_run: bool,
    /// One `[[backup.snapshots]]` entry, by index.
    pub(super) source: Option<usize>,
    #[serde(default)]
    to: String,
    /// A restore's newest snapshots not after this time.
    #[serde(default)]
    at: String,
    /// The repository a re-create copies from.
    #[serde(default)]
    from: String,
}

/// Starts the runs and reports them. If it started one run, the page opens it.
pub(super) async fn run(
    State(app): State<Arc<App>>,
    selection: Selection,
    base: Base,
    Form(form): Form<RunForm>,
) -> Result {
    let opt = |s: String| Some(s.trim().to_owned()).filter(|s| !s.is_empty());
    let selection = match (selection, opt(form.repo)) {
        (Selection::Collection(collection), Some(repo)) => Selection::Repo(collection, repo),
        (selection, _) => selection,
    };
    let to = opt(form.to).map(PathBuf::from);
    if to.as_ref().is_some_and(|t| !t.is_absolute()) {
        return Err(Error::Refused(Notice::DestinationNotAbsolute));
    }
    let snapshot = opt(form.snapshot)
        .map(|id| id.parse())
        .transpose()
        .map_err(|e: NotASnapshotId| Error::Refused(Notice::InvalidSnapshot(e.to_string())))?;
    let at = opt(form.at).map(|at| crate::engine::parse_at(&at)).transpose();
    let at = at.map_err(|e| Error::Refused(Notice::InvalidTime(format!("{e:#}"))))?;
    let options = RestoreOptions {
        snapshot,
        path: opt(form.path),
        source: form.source,
        at,
        to,
        from: opt(form.from),
        ..RestoreOptions::default()
    };
    // As on the command line: without choices, a dry run reads the snapshots as a restore would; with them, it
    // says what would be restored where.
    let choices = options.at.is_some() || options.to.is_some() || options.path.is_some();
    let tests = form.kind == RunKind::Restore && form.dry_run && !choices;
    let kind = if tests { RunKind::TestRestore } else { form.kind };
    let options = RestoreOptions { dry_run: form.dry_run && choices, ..options };
    Ok(blocking(move || {
        let submitted = app.submit(&selection, kind, &options).map_err(|e| {
            Error::Refused(Notice::NotSubmitted { collection: whose(&app, &selection), error: format!("{e:#}") })
        })?;
        let started = match selection {
            Selection::All if !submitted.started.is_empty() => vec![kind.doing()],
            Selection::All | Selection::Collection(_) | Selection::Repo(..) => {
                submitted.started.iter().map(|(.., doing)| doing.clone()).collect()
            }
        };
        let skipped = match (&selection, submitted.skipped.as_slice()) {
            (_, []) => None,
            // For every collection: the count.
            (Selection::All, all) => Some(all.len().to_string()),
            (Selection::Collection(_) | Selection::Repo(..), skipped) => Some(skipped.join(", ")),
        };
        let started = (!started.is_empty()).then(|| started.join(", "));
        let what: Vec<String> =
            started.into_iter().chain(skipped.map(|s| Label::AlreadyQueued { what: &s }.to_string())).collect();
        let response = flash(&Notice::Started { collection: whose(&app, &selection), what: what.join("; ") });
        match submitted.started.as_slice() {
            [(collection, id, _)] => {
                let run = base.to(Route::Run(collection, *id));
                let location = [(header::LOCATION, run.clone()), (HeaderName::from_static("hx-location"), run)];
                Ok((StatusCode::CREATED, location, response).into_response())
            }
            _ => Ok(response),
        }
    })
    .await)
}

pub(super) async fn refresh(State(app): State<Arc<App>>, selection: Selection) -> Response {
    blocking(move || match app.refresh(&selection) {
        Ok(()) => Ok(flash(&match &selection {
            Selection::Repo(collection, repo) => {
                Notice::RefreshingRepo { collection: app.engine().title(collection), repo: repo.clone() }
            }
            Selection::All | Selection::Collection(_) => Notice::Refreshing { collection: whose(&app, &selection) },
        })),
        Err(e) => Err(Error::Refused(Notice::NotRefreshed(format!("{e:#}")))),
    })
    .await
}

/// Saves the first collection, which the watcher then reads, and shows its profiles to keep a copy of.
pub(super) async fn save_setup(base: Base, Here(here): Here, Form(setup): Form<first_run::Setup>) -> Response {
    blocking(move || match first_run::save(&setup) {
        Ok(dir) => {
            let saved = SetupSaved {
                dir: dir.display().to_string(),
                here,
                restore: crate::args::command_line("restore", &[("all", None)]),
                resume: crate::args::command_line("install", &[("auto_update", None)]),
            };
            Ok(axum::response::Html(render_str(&saved, &base)).into_response())
        }
        Err(e) => Err(Error::Refused(Notice::SetupNotSaved { error: format!("{e:#}") })),
    })
    .await
}

/// Opens the profiles' folder in the file manager, for a browser on this computer, so that its user copies
/// them somewhere safe, or a fresh install's user puts them there. Elsewhere, there is no such page.
#[expect(clippy::disallowed_methods, reason = "creates the folder on a fresh install")]
pub(super) async fn open_profile_folder(Here(here): Here) -> Response {
    match crate::profile::own_dir().filter(|_| here) {
        Some(dir)
            if std::fs::create_dir_all(&dir).is_ok() && crate::server::open_on_desktop(dir.display().to_string()) =>
        {
            StatusCode::NO_CONTENT.into_response()
        }
        Some(_) => Error::Refused(Notice::FolderNotOpened).into_response(),
        None => Error::NotFound(Missing::Page).into_response(),
    }
}

/// Stops a run, or every run of the collection.
pub(super) async fn cancel(State(app): State<Arc<App>>, _: Selection, Path(p): Path<RunPath>) -> Response {
    blocking(move || match app.cancel(&p.collection, p.id) {
        Ok(0) if p.id.is_some() => {
            Err(Error::Refused(Notice::NotRunningHere { collection: app.engine().title(&p.collection) }))
        }
        Ok(_) => Ok(flash(&Notice::Stopping { collection: app.engine().title(&p.collection) })),
        Err(e) => Ok(flash(&Notice::CannotStop { error: format!("{e:#}") })),
    })
    .await
}

#[derive(Deserialize)]
pub(super) struct StepPath {
    collection: String,
    id: i64,
    step: usize,
}

/// Stops a step: the run goes on with the steps that do not need it.
pub(super) async fn cancel_step(State(app): State<Arc<App>>, _: Selection, Path(p): Path<StepPath>) -> Response {
    blocking(move || match app.cancel_step(&p.collection, p.id, p.step) {
        true => Ok(flash(&Notice::Stopping { collection: app.engine().title(&p.collection) })),
        false => Err(Error::Refused(Notice::NotRunningHere { collection: app.engine().title(&p.collection) })),
    })
    .await
}

#[derive(Deserialize)]
pub(super) struct NotifyTest {
    notify: String,
}

/// Sends a test to the notification URLs that the setup form holds.
pub(super) async fn test_notification(Form(form): Form<NotifyTest>) -> Response {
    blocking(move || match crate::notification::test(&form.notify) {
        Ok(()) => Ok(flash(&Notice::TestNotified)),
        Err(e) => Err(Error::Refused(Notice::TestNotNotified { error: e.to_string() })),
    })
    .await
}

/// Sends the collection's status to its notification URLs now.
pub(super) async fn notify(State(app): State<Arc<App>>, _: Selection, Path(collection): Path<String>) -> Response {
    blocking(move || {
        let engine = app.engine();
        let sent = engine
            .collection(&collection)
            .map(|j| (j.title.to_owned(), engine.notify_now(&j, &mut crate::notification::logged(Some(j.title)))));
        app.pages.changed_of(&collection);
        match sent {
            Some((collection, Ok(_))) => Ok(flash(&Notice::Notified { collection })),
            Some((collection, Err(e))) => Err(Error::Refused(Notice::NotNotified { collection, error: e.to_string() })),
            None => Err(Error::NotFound(Missing::Named(collection))),
        }
    })
    .await
}

/// Clears an issue of a step's last attempt, if the collection shows it.
pub(super) async fn dismiss(
    State(app): State<Arc<App>>,
    _: Selection,
    Path((collection, key)): Path<(String, String)>,
) -> Response {
    blocking(move || {
        let engine = app.engine();
        match key.parse().and_then(|key| engine.dismiss(&collection, key)) {
            Ok(true) => {
                app.pages.changed_of(&collection);
                Ok(flash(&Notice::Dismissed { collection: engine.title(&collection) }))
            }
            Ok(false) => Err(Error::Refused(Notice::NothingToDismiss { collection: engine.title(&collection) })),
            Err(e) => Ok(flash(&Notice::CannotDismiss { error: format!("{e:#}") })),
        }
    })
    .await
}
