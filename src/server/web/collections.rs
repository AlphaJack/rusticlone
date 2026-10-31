//! The overview, and each collection's page.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use askama::Template;
use axum::{
    extract::{Path, State},
    response::Response,
};

use super::{
    Access, Age, App, Base, Button, Error, Heading, Hint, Label, LiveQuery, Missing, Route, RunPath, Size, blocking,
    layout, live, now, render_str,
    runs::{LogView, RunRow, collection_run_rows, log_view},
};
use crate::{
    engine::{
        Blocked, Collection, CollectionOverview, Engine, Issue, RepoOverview, RepoStatus, Schedule, Unconfigured,
    },
    first_run, fmt,
    log::{Level, Message},
    profile,
    profile::{RusticProfile, Sources},
    result::{Detail, StepResult},
    rustic::{self, Snapshot},
    server::listing::Listing,
};

/// Runs on the overview.
const RECENT_RUNS: usize = 20;

pub(super) struct RepoRow {
    pub(super) name: String,
    primary: bool,
    pub(super) location: String,
    /// Reached over a network.
    pub(super) remote: bool,
    pub(super) class: &'static str,
    pub(super) status: RepoStatus,
    snapshots: String,
    size: Size,
    /// The primary's last backup, or a target's last copy.
    pub(super) backed_up: Age,
    pub(super) checked: Age,
    pub(super) pruned: Age,
    pub(super) backup_schedule: ScheduleView,
    pub(super) check_schedule: ScheduleView,
    pub(super) prune_schedule: ScheduleView,
    listing: bool,
    /// Measured before the last change.
    size_outdated: bool,
    /// Why Prune is disabled.
    prune_blocked: Option<String>,
    /// What a run of the collection is doing to it: "copying".
    doing: Option<String>,
    exists: bool,
    /// It exists, and was not listed without snapshots.
    pub(super) restorable: bool,
    /// A copy would bring it up to date: a target that lacks snapshots, or a primary with such a target.
    copy_helps: bool,
}

/// Whether a target lacks snapshots that a copy sends. A missing one is re-created instead, and one never
/// measured is refreshed first.
fn lacks(status: &RepoStatus) -> bool {
    matches!(status, RepoStatus::Behind { .. } | RepoStatus::Stale { .. } | RepoStatus::CopyFailing)
}

/// A schedule's icon: lit when scheduled, red when invalid. Its tooltip shows the expression and
/// the next run.
pub(super) struct ScheduleView {
    /// `on`, `failure`, or empty.
    pub(super) class: &'static str,
    pub(super) title: String,
}

impl ScheduleView {
    pub(super) fn new(what: Label<'_>, schedule: Option<&Schedule>) -> Self {
        let what = what.to_string();
        match schedule {
            Some(Schedule { error: Some(e), .. }) => {
                ScheduleView { class: "failure", title: Label::Unschedulable { what: &what, error: e }.to_string() }
            }
            Some(Schedule { cron, next: Some(next), .. }) => ScheduleView {
                class: "on",
                title: Label::ScheduledNext { what: &what, cron, next: &fmt::local(*next) }.to_string(),
            },
            Some(Schedule { cron, next: None, .. }) => {
                ScheduleView { class: "on", title: Label::Scheduled { what: &what, cron }.to_string() }
            }
            None => ScheduleView { class: "", title: Label::NotScheduled { what: &what }.to_string() },
        }
    }
}

/// A collection, as the overview and its page show it.
pub(super) struct CollectionView {
    name: String,
    title: String,
    copy_blocked: Option<Blocked>,
    /// Why Restore is disabled: no repository has snapshots.
    restore_blocked: Option<Hint<'static>>,
    level: Level,
    /// Its primary is being listed for the first time, so its level is not known yet.
    listing: bool,
    pub(super) repos: Vec<RepoRow>,
    issues: Vec<Issue>,
    /// Readable: it has a page and actions.
    configured: bool,
    sources: Vec<SourceView>,
    file: Option<String>,
    /// It has notification URLs.
    notifies: bool,
}

/// One `[[backup.snapshots]]` entry and its size at the last backup, from the primary's listing,
/// without scanning the sources.
struct SourceView {
    sources: Vec<String>,
    files: String,
    size: Size,
}

/// Everything a collection's view needs from the engine and the store, so that building the view reads
/// nothing else. Tests build views from it directly.
pub(super) struct CollectionInput {
    pub(super) overview: CollectionOverview,
    /// Newest first.
    pub(super) runs: Vec<RunRow>,
    /// By repository name.
    pub(super) listings: BTreeMap<String, Listing>,
    /// Its repositories that exist.
    pub(super) existing: BTreeSet<String>,
    /// Its repositories reached over a network.
    pub(super) remote: BTreeSet<String>,
    /// Its repositories without a `[forget]` policy, which keep every snapshot.
    pub(super) kept: BTreeSet<String>,
    /// Its collection's file.
    pub(super) file: Option<String>,
    /// It has notification URLs.
    pub(super) notifies: bool,
    /// Readable. Otherwise the collection is broken and has no repositories or actions.
    pub(super) configured: bool,
    pub(super) copy_blocked: Option<Blocked>,
    pub(super) sources: Vec<Sources>,
    /// The primary's snapshots, as last listed.
    pub(super) snapshots: Vec<Snapshot>,
    /// The primary was listed at least once. Until then, the collection's level is unknown.
    pub(super) listed: bool,
}

impl CollectionInput {
    /// `runs`: how many of its newest runs to show, besides those not over.
    pub(super) fn read(
        app: &App,
        engine: &Engine,
        collection: Option<&Collection<'_>>,
        overview: CollectionOverview,
        runs: usize,
    ) -> Self {
        let name = overview.primary.name.clone();
        let repos: Vec<&RusticProfile> = collection.into_iter().flat_map(Collection::repos).collect();
        let listings = repos.iter().filter_map(|r| Some((r.name.clone(), app.listing(&name, &r.name)?))).collect();
        let existing = repos.iter().filter(|r| r.exists()).map(|r| r.name.clone()).collect();
        let remote = repos.iter().filter(|r| r.is_remote()).map(|r| r.name.clone()).collect();
        let kept = repos.iter().filter(|r| !r.has_keep).map(|r| r.name.clone()).collect();
        let primary = collection.map(|j| j.primary);
        let sources = primary.into_iter().flat_map(|p| p.sources.clone()).collect();
        let inventory = engine.store.inventory(&name, &name);
        let listed = inventory.is_some();
        let snapshots = inventory.and_then(|l| l.result.ok()).map(|i| i.snapshots);
        let notifies = collection.is_some_and(|j| crate::settings::NOTIFY.get(&[j.primary]).is_some());
        CollectionInput {
            runs: collection_run_rows(engine, &name, runs),
            listings,
            existing,
            remote,
            kept,
            file: collection.and_then(|j| j.primary.files().first().map(|f| f.display().to_string())),
            notifies,
            configured: collection.is_some(),
            copy_blocked: collection.and_then(Collection::copy_blocked),
            sources,
            snapshots: snapshots.unwrap_or_default(),
            listed,
            overview,
        }
    }
}

/// A repository's row, from its last listing.
struct Listed {
    snapshots: String,
    size: Size,
    listing: bool,
    size_outdated: bool,
    restorable: bool,
}

impl Listed {
    /// `restorable` unless the listing says otherwise.
    fn of(listing: Option<&Listing>, restorable: bool) -> Self {
        let mut listed = Listed {
            snapshots: Label::Unknown.to_string(),
            size: Size(None),
            listing: false,
            size_outdated: false,
            restorable,
        };
        match listing {
            Some(Listing::Listed(l)) => {
                listed.size_outdated = l.size_outdated;
                match &l.result {
                    Ok(info) => {
                        listed.restorable = !info.snapshots.is_empty();
                        listed.snapshots = info.snapshots.len().to_string();
                        listed.size = Size(info.stored);
                    }
                    Err(_) => (listed.snapshots, listed.restorable) = (Label::Error.to_string(), false),
                }
            }
            Some(Listing::Running) => listed.listing = true,
            None => {}
        }
        listed
    }
}

impl CollectionView {
    pub(super) fn build(input: CollectionInput) -> Self {
        let CollectionInput {
            overview: s,
            runs: _,
            listings,
            existing,
            remote,
            kept,
            file,
            notifies,
            configured,
            copy_blocked,
            sources,
            snapshots,
            listed,
        } = input;
        let name = s.primary.name.clone();
        let any_lacks = s.targets.iter().any(|t| lacks(&t.status));
        let row = |r: &RepoOverview, primary: bool| {
            let on = s.running.iter().flat_map(|run| run.activities.iter().filter_map(|a| a.on.as_ref()));
            let doing = on.clone().find(|(repo, _)| *repo == r.name).map(|(_, doing)| doing.clone());
            // A missing repository's old listing no longer applies.
            let listed = Listed::of(listings.get(&r.name).filter(|_| !r.missing), !r.missing);
            RepoRow {
                copy_helps: if primary { any_lacks } else { lacks(&r.status) },
                name: r.name.clone(),
                primary,
                location: r.repository.clone().unwrap_or_default(),
                remote: remote.contains(&r.name),
                class: r.status.tone().into(),
                status: r.status,
                snapshots: listed.snapshots,
                size: listed.size,
                backed_up: Age::Last(s.backed_up(r)),
                checked: Age::Last(r.checked),
                pruned: Age::Last(r.pruned),
                backup_schedule: ScheduleView::new(
                    if primary { Label::BackupSchedule } else { Label::CopySchedule },
                    s.backup_schedule.as_ref(),
                ),
                check_schedule: ScheduleView::new(Label::CheckSchedule, r.check_schedule.as_ref()),
                prune_schedule: ScheduleView::new(Label::PruneSchedule, r.prune_schedule.as_ref()),
                prune_blocked: kept
                    .contains(&r.name)
                    .then(|| Detail(&StepResult::KeepsEverything { repo: r.name.clone() }).to_string()),
                listing: listed.listing,
                size_outdated: listed.size_outdated,
                doing,
                exists: existing.contains(&r.name),
                restorable: listed.restorable,
            }
        };
        let repos: Vec<RepoRow> = match configured {
            true => std::iter::once(row(&s.primary, true)).chain(s.targets.iter().map(|t| row(t, false))).collect(),
            false => Vec::new(),
        };
        let source = |s: Sources| {
            let group = rustic::source_key(&s.paths());
            let newest = snapshots.iter().filter(|sn| sn.source() == group).max_by_key(|sn| sn.time);
            let summary = newest.and_then(|sn| sn.summary.as_ref());
            SourceView {
                files: summary.map_or(Label::Unknown.to_string(), |s| s.total_files_processed.to_string()),
                size: Size(summary.map(|s| s.total_bytes_processed)),
                sources: s.sources,
            }
        };
        CollectionView {
            copy_blocked,
            restore_blocked: (!repos.is_empty() && !repos.iter().any(|r| r.restorable))
                .then_some(Hint::NothingToRestore),
            level: s.level,
            listing: configured && !listed,
            repos,
            issues: s.issues,
            configured,
            sources: sources.into_iter().map(source).collect(),
            file,
            notifies,
            name,
            title: s.title,
        }
    }
}

struct Overview {
    collections: Vec<CollectionView>,
    /// Some collection has actions.
    actions: bool,
}

/// No collections yet: where to put them, and examples.
struct Empty {
    dir: String,
    /// The setup form can save there.
    writable: bool,
}

#[derive(Template)]
#[template(path = "index.html")]
pub(super) struct IndexMain {
    overview: Overview,
    runs: Vec<RunRow>,
    with_collection: bool,
    empty: Option<Empty>,
    untested: Option<rustic::Untested>,
    /// [`crate::install::autostart::Blocked`].
    blocked: Option<String>,
    unconfigured: Option<Unconfigured>,
    waiting: Option<crate::engine::waiting::Waiting>,
}

pub(super) fn index_main(app: &App) -> IndexMain {
    let engine = app.engine();
    let inputs: Vec<CollectionInput> = engine
        .status()
        .into_iter()
        .map(|s| CollectionInput::read(app, &engine, engine.collection(&s.primary.name).as_ref(), s, RECENT_RUNS))
        .collect();
    let empty = inputs.is_empty().then(|| Empty {
        dir: profile::own_dir().map(|d| d.display().to_string()).unwrap_or_default(),
        writable: first_run::dir().is_some(),
    });
    #[cfg(feature = "desktop")]
    let blocked = crate::install::autostart::blocked().map(|b| b.to_string());
    #[cfg(not(feature = "desktop"))]
    let blocked = None;
    IndexMain {
        empty,
        untested: engine.rustic.untested(),
        blocked,
        unconfigured: engine.unconfigured(),
        waiting: crate::engine::waiting::waiting(true),
        ..IndexMain::build(inputs)
    }
}

impl IndexMain {
    pub(super) fn build(inputs: Vec<CollectionInput>) -> Self {
        let mut runs: Vec<RunRow> = inputs.iter().filter(|i| i.configured).flat_map(|i| i.runs.clone()).collect();
        runs.sort_by_key(|r| std::cmp::Reverse((r.waiting, r.id)));
        runs.truncate(RECENT_RUNS);
        let collections: Vec<CollectionView> = inputs.into_iter().map(CollectionView::build).collect();
        let actions = collections.iter().any(|j| j.configured);
        IndexMain {
            overview: Overview { collections, actions },
            runs,
            with_collection: true,
            empty: None,
            untested: None,
            blocked: None,
            unconfigured: None,
            waiting: None,
        }
    }
}

pub(super) async fn index(State(app): State<Arc<App>>, access: Access, base: Base) -> Response {
    blocking(move || {
        let live = live(&app, None, None, None);
        Ok(layout(&Heading::Collections.to_string(), Some(live), access, &base, render_str(&index_main(&app), &base)))
    })
    .await
}

#[derive(Template)]
#[template(path = "collection.html")]
pub(super) struct CollectionMain {
    collection: CollectionView,
    pub(super) log: LogView,
    runs: Vec<RunRow>,
    with_collection: bool,
}

pub(super) fn collection_main(
    app: &App,
    name: &str,
    picked: Option<i64>,
    shown: Option<(i64, u64)>,
) -> Option<CollectionMain> {
    let engine = app.engine();
    let collection = engine.collection(name)?;
    let input = CollectionInput::read(app, &engine, Some(&collection), engine.status_of(&collection), usize::MAX);
    let log = log_view(&engine, name, &input.runs, picked, shown);
    Some(CollectionMain::build(input, log))
}

impl CollectionMain {
    pub(super) fn build(input: CollectionInput, log: LogView) -> Self {
        let mut runs = input.runs.clone();
        for r in &mut runs {
            r.shown = log.shown().is_some_and(|(run, _)| r.id == run);
        }
        CollectionMain { log, runs, collection: CollectionView::build(input), with_collection: false }
    }
}

pub(super) async fn collection_page(
    State(app): State<Arc<App>>,
    access: Access,
    base: Base,
    Path(p): Path<RunPath>,
) -> Response {
    blocking(move || {
        let live = live(&app, Some(&p.collection), None, p.id);
        match collection_main(&app, &p.collection, p.id, None) {
            Some(main) if p.id.is_none_or(|id| main.log.shown().is_some_and(|(run, _)| run == id)) => Ok(layout(
                &main.collection.title,
                Some(LiveQuery { log: main.log.shown(), ..live }),
                access,
                &base,
                render_str(&main, &base),
            )),
            Some(_) => Err(Error::NotFound(Missing::Run)),
            None => Err(Error::NotFound(Missing::Collection)),
        }
    })
    .await
}
