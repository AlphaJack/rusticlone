//! A repository's page: its snapshots and size, as last listed.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, State},
    response::Response,
};

use super::{
    Access, App, Base, Button, Error, Heading, Hint, Label, Missing, Route, Size, blocking,
    collections::{CollectionInput, CollectionView, RepoRow},
    layout, live, render_str,
};
use crate::{
    engine::{Collection, Engine},
    fmt,
    profile::RusticProfile,
    rustic::SnapshotId,
    server::listing::Listing,
    state::Inventory,
};

pub(super) fn with_repo<T>(
    app: &App,
    collection: &str,
    repo: &str,
    f: impl FnOnce(&Engine, &Collection<'_>, &RusticProfile) -> T,
) -> Option<T> {
    let engine = app.engine();
    let j = engine.collection(collection)?;
    let p = j.repo(repo)?;
    Some(f(&engine, &j, p))
}

struct SnapshotRow {
    id: SnapshotId,
    short: String,
    at: String,
    host: String,
    paths: String,
    files: String,
    size: Size,
    added: Size,
}

pub(super) struct RepoView {
    collection: String,
    title: String,
    name: String,
    /// The collection's row for it: status and location.
    row: Option<RepoRow>,
    listing: bool,
    error: Option<String>,
    snapshots: Vec<SnapshotRow>,
}

#[derive(Template)]
#[template(path = "repo.html")]
pub(super) struct RepoMain {
    pub(super) repo: RepoView,
}

/// Runs keep the snapshots current, but not the size.
fn measuring(l: &Inventory) -> bool {
    l.size_outdated || l.result.as_ref().is_ok_and(|i| i.stored.is_none())
}

pub(super) fn repo_view(app: &Arc<App>, collection: &str, repo: &str) -> Option<RepoView> {
    let engine = app.engine();
    let j = engine.collection(collection).filter(|j| j.repo(repo).is_some())?;
    let input = CollectionInput::read(app, &engine, Some(&j), engine.status_of(&j), 0);
    // Never measured, or measured before the last change: measure it, and show the old size meanwhile.
    let stale = |i: &Listing| matches!(i, Listing::Listed(l) if measuring(l));
    if input.listings.get(repo).is_none_or(stale) {
        app.list_repos(collection, vec![repo.to_owned()]);
    }
    Some(RepoView::build(input, repo))
}

impl RepoView {
    pub(super) fn build(input: CollectionInput, repo: &str) -> Self {
        let (collection, title) = (input.overview.primary.name.clone(), input.overview.title.clone());
        let inventory = match input.listings.get(repo) {
            Some(Listing::Listed(l)) => Some(l),
            Some(Listing::Running) | None => None,
        };
        let mut view = RepoView {
            collection,
            title,
            name: repo.to_owned(),
            row: None,
            listing: inventory.is_none_or(measuring),
            error: None,
            snapshots: Vec::new(),
        };
        if let Some(l) = inventory {
            match &l.result {
                Err(e) => view.error = Some(e.clone()),
                Ok(info) => {
                    let mut snapshots: Vec<_> = info.snapshots.iter().collect();
                    snapshots.sort_by_key(|s| std::cmp::Reverse(s.time));
                    view.snapshots = snapshots
                        .into_iter()
                        .map(|s| {
                            let summary = s.summary.clone().unwrap_or_default();
                            SnapshotRow {
                                short: s.id.short().to_owned(),
                                at: fmt::local(s.time),
                                host: s.hostname.clone(),
                                paths: s.paths.join(", "),
                                files: summary.total_files_processed.to_string(),
                                size: Size(Some(summary.total_bytes_processed)),
                                added: Size(Some(summary.data_added_packed)),
                                id: s.id.clone(),
                            }
                        })
                        .collect();
                }
            }
        }
        view.row = CollectionView::build(input).repos.into_iter().find(|r| r.name == repo);
        view
    }
}

pub(super) async fn repo_page(
    State(app): State<Arc<App>>,
    access: Access,
    base: Base,
    Path((collection, repo)): Path<(String, String)>,
) -> Response {
    blocking(move || match (live(&app, Some(&collection), Some(&repo), None), repo_view(&app, &collection, &repo)) {
        (live, Some(view)) => {
            let title = format!("{} | {}", view.name, view.title);
            let main = render_str(&RepoMain { repo: view }, &base);
            Ok(layout(&title, Some(live), access, &base, main))
        }
        (_, None) => Err(Error::NotFound(Missing::Repository)),
    })
    .await
}
