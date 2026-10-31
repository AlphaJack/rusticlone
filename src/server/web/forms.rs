//! Forms for actions that need input: restore, test restore, re-create, and the first collection.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{Html, IntoResponse, Response},
};
use strum::IntoEnumIterator as _;

use super::{
    Access, App, Base, Button, Error, Heading, Hint, Label, Missing, Route, actions::RunForm, blocking, layout,
    render_str,
};
use crate::{
    engine::Collection,
    first_run::{self, Choice, Every, Keep, Kind, Question, Role, Setup},
    request::RunKind,
};

/// The first collection's dialog, with a new password.
#[derive(Template)]
#[template(path = "setup.html")]
pub(super) struct SetupFormView {
    setup: Setup,
    everies: Vec<Every>,
    keeps: Vec<Keep>,
    notifiers: Vec<Notifier>,
}

/// A toggle that adds a notification service's URL to the field, or removes it. It is on while the field has
/// a URL of one of its schemes, as the page keeps it.
struct Notifier {
    name: String,
    /// Space-separated.
    schemes: String,
    /// The URL it adds, with its `{parts}` to fill in.
    template: String,
    on: bool,
}

/// Services whose URLs only name an endpoint that takes a request, for those who write their own.
const GENERIC: [&str; 4] = ["appriseapi", "form", "json", "xml"];

impl SetupFormView {
    pub(super) fn new(setup: Setup) -> Self {
        let notifiers = Notifier::all(&setup.notify);
        let (everies, keeps) = (Every::iter().collect(), Keep::iter().collect());
        Self { setup, everies, keeps, notifiers }
    }
}

impl Notifier {
    /// This desktop's, then each compiled-in service but the generic ones and other desktops, which take no
    /// parts, by name.
    fn all(notify: &str) -> Vec<Self> {
        let toggle = |name: String, def: &mercurio::service::ServiceDef, template: String| {
            let on = notify
                .split_whitespace()
                .filter_map(|url| url.split_once("://"))
                .any(|(s, _)| def.schemes.iter().any(|own| own.eq_ignore_ascii_case(s)));
            Self { name, schemes: def.schemes.join(" "), template, on }
        };
        let desktop = first_run::desktop().and_then(|url| {
            let def = mercurio::service_for_scheme(url.split_once("://")?.0)?;
            Some(toggle(Heading::Desktop.to_string(), def, url.to_owned()))
        });
        let mut services: Vec<Self> = (mercurio::services().iter())
            .filter(|def| !GENERIC.contains(&def.id) && def.templates.iter().all(|t| t.contains("://{")))
            .filter_map(|def| {
                // The secure scheme, listed last.
                let template = def.templates.first()?.replace("{schema}", def.schemes.last()?);
                Some(toggle(def.name.to_owned(), def, template))
            })
            .collect();
        services.sort_by_key(|n| n.name.to_lowercase());
        desktop.into_iter().chain(services).collect()
    }
}

/// What a custom schedule's fields mean.
const CRON_HELP: &str = "https://crontab.guru/";
/// A page of its own, without live updates, which would reset what is typed. Without a folder to save in,
/// the collections page says where to write profiles instead.
pub(super) async fn setup(access: Access, base: Base) -> Response {
    blocking(move || {
        if first_run::dir().is_none() {
            return Ok(axum::response::Redirect::to(&base.to(Route::Index)).into_response());
        }
        let view = SetupFormView::new(Setup::default());
        Ok(layout(&Heading::SetUp.to_string(), None, access, &base, render_str(&view, &base)))
    })
    .await
}

/// The restore and test restore dialog.
#[derive(Template)]
#[template(path = "restore.html")]
struct RestoreFormView {
    collection: String,
    /// The dry run checked, as a test restore's link asks.
    dry_run: bool,
    /// Each repository to choose from.
    repos: Vec<String>,
    from: String,
    snapshot: String,
    short: String,
    /// The paths it restores as rustic recorded them, when the page that opened it chose them: one path, or
    /// those of one snapshot. Empty to choose in the dialog.
    what: Vec<String>,
    path: String,
    source: Option<usize>,
    /// Each backup entry's folders to choose from, by entry.
    entries: Vec<String>,
}

impl RestoreFormView {
    /// Whether the page that opened it chose backup entry `entry`, as a source's row does.
    #[expect(clippy::trivially_copy_pass_by_ref, reason = "the template passes its loop's index by reference")]
    fn picked(&self, entry: &usize) -> bool {
        self.source == Some(*entry)
    }
}

/// The re-create dialog: which repository to copy from.
#[derive(Template)]
#[template(path = "recreate.html")]
struct RecreateFormView {
    collection: String,
    repo: String,
    /// What the default choice does.
    default: Hint<'static>,
    /// Each other repository it can be copied from.
    sources: Vec<String>,
}

/// The form of a run, in the page's dialog, or as a page of its own:
/// - restore: the newest snapshots of the sources, or one snapshot, or one path in it;
/// - test restore: the newest snapshot of a source, from the primary unless another repository is
///   chosen, or one snapshot;
/// - re-create: which repository to copy from.
pub(super) async fn form(
    State(app): State<Arc<App>>,
    access: Access,
    base: Base,
    headers: HeaderMap,
    Path(collection): Path<String>,
    Query(q): Query<RunForm>,
) -> Response {
    blocking(move || {
        let title = match q.kind {
            RunKind::Recreate => Button::Recreate,
            RunKind::Restore
            | RunKind::TestRestore
            | RunKind::Backup
            | RunKind::Copy
            | RunKind::Prune
            | RunKind::Check => Button::Restore,
        };
        let title = format!("{title} | {}", app.engine().title(&collection));
        let main = if q.kind == RunKind::Recreate {
            render_str(&recreate_form(&app, collection, &q.repo).map_err(Error::NotFound)?, &base)
        } else {
            render_str(&restore_form(&app, collection, q).map_err(Error::NotFound)?, &base)
        };
        Ok(if headers.contains_key("hx-request") {
            Html(main).into_response()
        } else {
            layout(&title, None, access, &base, main)
        })
    })
    .await
}

/// Fails with what was not found.
fn recreate_form(app: &App, collection: String, repo: &str) -> Result<RecreateFormView, Missing> {
    let engine = app.engine();
    let j = engine.collection(&collection).ok_or(Missing::Collection)?;
    let repo = j.repo(repo).ok_or(Missing::Repository)?.name.clone();
    let primary = repo == collection;
    let sources = repos(&j).into_iter().filter(|name| *name != repo && *name != collection).collect::<Vec<_>>();
    let default = match (primary, sources.is_empty()) {
        (true, true) => Hint::CreatedEmpty,
        (true, false) => Hint::NewestTarget,
        (false, _) => Hint::FromPrimary,
    };
    Ok(RecreateFormView { collection, repo, default, sources })
}

/// Fails with what was not found.
fn restore_form(app: &App, collection: String, q: RunForm) -> Result<RestoreFormView, Missing> {
    let dry_run = match q.kind {
        RunKind::Restore => false,
        RunKind::TestRestore => true,
        RunKind::Backup | RunKind::Copy | RunKind::Prune | RunKind::Check | RunKind::Recreate => {
            return Err(Missing::RunKind);
        }
    };
    let engine = app.engine();
    let j = engine.collection(&collection).ok_or(Missing::Collection)?;
    let snapshot_paths = || {
        let listed = engine.store.snapshots(&collection, &q.repo);
        Some(listed.into_iter().find(|sn| sn.id.as_str() == q.snapshot)?.paths)
    };
    let what = match (q.path.is_empty(), q.snapshot.is_empty()) {
        (false, _) => vec![q.path.clone()],
        (true, false) => snapshot_paths().unwrap_or_default(),
        (true, true) => Vec::new(),
    };
    let entries = j.primary.groups().map(|paths| paths.join(", ")).collect();
    Ok(RestoreFormView {
        dry_run,
        from: q.repo,
        repos: repos(&j),
        collection,
        short: crate::rustic::short_id(&q.snapshot).to_owned(),
        snapshot: q.snapshot,
        what,
        path: q.path,
        source: q.source,
        entries,
    })
}

fn repos(j: &Collection<'_>) -> Vec<String> {
    j.repos().map(|p| p.name.clone()).collect()
}
