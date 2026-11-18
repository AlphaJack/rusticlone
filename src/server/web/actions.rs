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
    App, Base, Button, Error, Heading, Hint, Label, Missing, Notice, Result, Route, RunPath, auth::LocalBrowser,
    blocking, flash, render_str,
};
use crate::{
    request::{RestoreOptions, RunKind, Selection},
    rustic::NotASnapshotId,
    setup,
};

/// The selection that the path names, if it exists: `/runs`, `/collections/{collection}/runs`,
/// `/collections/{collection}/repos/{repo}/runs`.
impl FromRequestParts<Arc<App>> for Selection {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, app: &Arc<App>) -> Result<Self, Response> {
        let params = parts.extract::<RawPathParams>().await.ok();
        let param = |name: &str| Some(params.as_ref()?.iter().find(|(k, _)| *k == name)?.1.to_owned());
        let selection = match (param("collection"), param("repo")) {
            (Some(collection), Some(repo)) => Self::Repo(collection, repo),
            (Some(collection), None) => Self::Collection(collection),
            (None, _) => Self::All,
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

/// The guide to restoring on a new computer, which starts with installing: the link the emergency kit gives too.
const RESTORE_GUIDE: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "#restore");

/// Where the profiles were saved, and how to restore from them on a new computer.
#[derive(Template)]
#[template(path = "saved.html")]
pub(super) struct SetupSaved {
    pub(super) dir: String,
    /// A browser on this computer, which can open their folder.
    pub(super) local_browser: bool,
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
    let (selection, kind, options) = form.asked(selection).map_err(Error::Refused)?;
    Ok(blocking(move || {
        let submitted = app.submit(&selection, kind, &options).map_err(|e| {
            Error::Refused(Notice::NotSubmitted { collection: whose(&app, &selection), error: format!("{e:#}") })
        })?;
        let what = reported(&selection, kind, &submitted);
        let response = flash(&Notice::Started { collection: whose(&app, &selection), what });
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

impl RunForm {
    /// What the form asks for: the selection, narrowed to the repository it names, the kind of run, and the
    /// restore's options. Refused when a field cannot be read.
    fn asked(self, selection: Selection) -> std::result::Result<(Selection, RunKind, RestoreOptions), Notice> {
        let opt = |s: String| Some(s.trim().to_owned()).filter(|s| !s.is_empty());
        let selection = match (selection, opt(self.repo)) {
            (Selection::Collection(collection), Some(repo)) => Selection::Repo(collection, repo),
            (selection, _) => selection,
        };
        let to = opt(self.to).map(PathBuf::from);
        if to.as_ref().is_some_and(|t| !t.is_absolute()) {
            return Err(Notice::DestinationNotAbsolute);
        }
        let snapshot = (opt(self.snapshot).map(|id| id.parse()).transpose())
            .map_err(|e: NotASnapshotId| Notice::InvalidSnapshot(e.to_string()))?;
        let at = opt(self.at).map(|at| crate::request::parse_at(&at)).transpose();
        let at = at.map_err(|e| Notice::InvalidTime(format!("{e:#}")))?;
        let options = RestoreOptions {
            snapshot,
            path: opt(self.path),
            source: self.source,
            at,
            to,
            from: opt(self.from),
            ..RestoreOptions::default()
        };
        // As on the command line: without choices, a dry run reads the snapshots as a restore would; with them, it
        // says what would be restored where.
        let choices = options.at.is_some() || options.to.is_some() || options.path.is_some();
        let tests = self.kind == RunKind::Restore && self.dry_run && !choices;
        let kind = if tests { RunKind::TestRestore } else { self.kind };
        Ok((selection, kind, RestoreOptions { dry_run: self.dry_run && choices, ..options }))
    }
}

/// What the notice says of the runs `submitted` started and skipped: for every collection, what they do and how many
/// were already queued; for one, each run and each label.
fn reported(selection: &Selection, kind: RunKind, submitted: &crate::server::workers::Submitted) -> String {
    let started = match selection {
        Selection::All if !submitted.started.is_empty() => vec![kind.doing()],
        Selection::All | Selection::Collection(_) | Selection::Repo(..) => {
            submitted.started.iter().map(|(.., doing)| doing.clone()).collect()
        }
    };
    let skipped = match (selection, submitted.skipped.as_slice()) {
        (_, []) => None,
        // For every collection: the count.
        (Selection::All, all) => Some(all.len().to_string()),
        (Selection::Collection(_) | Selection::Repo(..), skipped) => Some(skipped.join(", ")),
    };
    let started = (!started.is_empty()).then(|| started.join(", "));
    let what: Vec<String> =
        started.into_iter().chain(skipped.map(|s| Label::AlreadyQueued { what: &s }.to_string())).collect();
    what.join("; ")
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
pub(super) async fn save_setup(
    base: Base,
    LocalBrowser(local_browser): LocalBrowser,
    Form(setup): Form<setup::Setup>,
) -> Response {
    blocking(move || match setup::save(&setup) {
        Ok(dir) => {
            let saved = SetupSaved {
                dir: dir.display().to_string(),
                local_browser,
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
/// them somewhere safe. Elsewhere, there is no such page.
pub(super) async fn open_profile_folder(LocalBrowser(local_browser): LocalBrowser) -> Response {
    match crate::profile::own_dir().filter(|_| local_browser) {
        Some(dir) if crate::server::open_on_desktop(&dir.display().to_string()) => {
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

#[derive(Deserialize)]
pub(super) struct OpenForm {
    repo: Option<String>,
    source: Option<String>,
}

/// Shows what the page names selected in its folder in the file manager, for a browser on this computer: a
/// repository's profile (the file users edit) or a source. Its path comes from the profile. The file manager only
/// shows it, so a page never opens a file or a program. Elsewhere the page copies the path instead.
#[expect(clippy::disallowed_methods, reason = "shows only a source that exists")]
pub(super) async fn open_collection_folder(
    State(app): State<Arc<App>>,
    LocalBrowser(local_browser): LocalBrowser,
    Path(collection): Path<String>,
    Form(form): Form<OpenForm>,
) -> Response {
    let engine = app.engine();
    let Some(j) = engine.collection(&collection).filter(|_| local_browser) else {
        return Error::NotFound(Missing::Page).into_response();
    };
    let shown = match (&form.repo, &form.source) {
        (Some(repo), _) => j.repo(repo).and_then(|p| p.files().into_iter().next()),
        (None, Some(source)) => (|| {
            let (entry, item) = source.split_once('/')?;
            let entry = j.primary.sources.get(entry.parse::<usize>().ok()?).filter(|s| s.in_place())?;
            Some(PathBuf::from(entry.sources.get(item.parse::<usize>().ok()?)?)).filter(|p| p.exists())
        })(),
        (None, None) => None,
    };
    let Some(shown) = shown else { return Error::NotFound(Missing::Page).into_response() };
    drop(engine);
    if crate::server::reveal_on_desktop(&shown).await {
        StatusCode::NO_CONTENT.into_response()
    } else {
        Error::Refused(Notice::FolderNotOpened).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::workers::Submitted;

    fn form(kind: RunKind) -> RunForm {
        RunForm {
            kind,
            repo: String::new(),
            snapshot: String::new(),
            path: String::new(),
            dry_run: false,
            source: None,
            to: String::new(),
            at: String::new(),
            from: String::new(),
        }
    }

    fn docs() -> Selection {
        Selection::Collection("Docs".into())
    }

    /// A repository narrows a collection, and the fields are read without their spaces.
    #[test]
    fn a_form_names_what_to_run_and_how() {
        let named =
            RunForm { repo: " Docs-usb ".into(), source: Some(1), from: " Docs ".into(), ..form(RunKind::Recreate) };
        let (selection, kind, options) = named.asked(docs()).unwrap();
        assert_eq!((selection, kind), (Selection::Repo("Docs".into(), "Docs-usb".into()), RunKind::Recreate));
        assert_eq!((options.source, options.from.as_deref()), (Some(1), Some("Docs")));
        let every = RunForm { repo: "Docs-usb".into(), ..form(RunKind::Backup) };
        assert_eq!(every.asked(Selection::All).unwrap().0, Selection::All, "no collection to narrow");
    }

    /// A dry run of a restore without choices reads the snapshots as a test restore. With a choice, it says what it
    /// would restore where.
    #[test]
    fn a_dry_run_restore_tests_unless_it_has_choices() {
        let dry = || RunForm { dry_run: true, ..form(RunKind::Restore) };
        let (_, kind, options) = dry().asked(docs()).unwrap();
        assert_eq!((kind, options.dry_run), (RunKind::TestRestore, false));
        let to = std::env::temp_dir().display().to_string();
        for chosen in [
            RunForm { path: "a".into(), ..dry() },
            RunForm { to, ..dry() },
            RunForm { at: "2026-10-16".into(), ..dry() },
        ] {
            let (_, kind, options) = chosen.asked(docs()).unwrap();
            assert_eq!((kind, options.dry_run), (RunKind::Restore, true));
        }
        let (_, kind, options) = form(RunKind::Restore).asked(docs()).unwrap();
        assert_eq!((kind, options.dry_run), (RunKind::Restore, false));
        let (_, kind, _) = RunForm { dry_run: true, ..form(RunKind::Check) }.asked(docs()).unwrap();
        assert_eq!(kind, RunKind::Check, "only a restore tests");
    }

    /// A relative destination, a snapshot id or a time that cannot be read refuse the form.
    #[test]
    fn a_form_that_cannot_be_read_is_refused() {
        let refused = |form: RunForm| form.asked(docs()).err();
        assert!(matches!(
            refused(RunForm { to: "restored".into(), ..form(RunKind::Restore) }),
            Some(Notice::DestinationNotAbsolute)
        ));
        assert!(matches!(
            refused(RunForm { snapshot: "zz".into(), ..form(RunKind::Restore) }),
            Some(Notice::InvalidSnapshot(_))
        ));
        assert!(matches!(
            refused(RunForm { at: "soon".into(), ..form(RunKind::Restore) }),
            Some(Notice::InvalidTime(_))
        ));
    }

    /// For every collection, the notice says what started once and how many were queued already. For one, it names
    /// each run and each one queued.
    #[test]
    fn the_notice_names_what_started() {
        let submitted = |started: &[&str], skipped: &[&str]| Submitted {
            started: started.iter().zip(1..).map(|(doing, id)| ("Docs".into(), id, (*doing).into())).collect(),
            skipped: skipped.iter().map(|s| (*s).into()).collect(),
        };
        let texts = [
            reported(
                &Selection::All,
                RunKind::Backup,
                &submitted(&["backing up Docs", "backing up Pics"], &["backup Mail"]),
            ),
            reported(&Selection::All, RunKind::Backup, &submitted(&[], &["backup Mail", "backup Pics"])),
            reported(&docs(), RunKind::Check, &submitted(&["checking Docs", "checking Docs-usb"], &["prune Docs"])),
            reported(&docs(), RunKind::Check, &submitted(&[], &[])),
        ];
        insta::assert_debug_snapshot!(texts);
    }
}
