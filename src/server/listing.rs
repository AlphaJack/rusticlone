//! Repositories listed on demand in the background: their snapshots and sizes, stored with the
//! collection's runs.

use std::{collections::BTreeSet, sync::Arc};

use anyhow::Result;
use parking_lot::Mutex;

use super::{App, Fault};
use crate::{request::Selection, state::Inventory};

pub(super) enum Listing {
    Running,
    Listed(Inventory),
}

/// Repositories being listed, by collection.
#[derive(Default)]
pub(super) struct Listings(Mutex<BTreeSet<(String, String)>>);

impl Listings {
    fn running(&self, collection: &str, repo: &str) -> bool {
        self.0.lock().contains(&(collection.to_owned(), repo.to_owned()))
    }

    /// Returns false if it is already being listed.
    fn start(&self, collection: &str, repo: &str) -> bool {
        self.0.lock().insert((collection.to_owned(), repo.to_owned()))
    }

    fn done(&self, collection: &str, repo: &str) {
        self.0.lock().remove(&(collection.to_owned(), repo.to_owned()));
    }
}

impl App {
    pub(super) fn listing(&self, collection: &str, repo: &str) -> Option<Listing> {
        if self.listing.running(collection, repo) {
            return Some(Listing::Running);
        }
        self.engine().store.inventory(collection, repo).map(Listing::Listed)
    }

    /// Lists every repository that `selection` names again.
    pub(super) fn refresh(self: &Arc<Self>, selection: &Selection) -> Result<()> {
        for (j, repo) in self.engine().named(selection)? {
            let repos = repo.map_or_else(|| j.repos().map(|p| p.name.clone()).collect(), |p| vec![p.name.clone()]);
            self.list_repos(j.name(), repos);
        }
        Ok(())
    }

    /// Lists the repositories in the background, one after the other.
    pub(super) fn list_repos(self: &Arc<Self>, collection: &str, repos: Vec<String>) {
        let repos: Vec<String> = repos.into_iter().filter(|r| self.listing.start(collection, r)).collect();
        self.pages.changed_of(collection);
        let (app, collection) = (self.clone(), collection.to_owned());
        tokio::task::spawn_blocking(move || {
            let engine = app.engine();
            let listed = |repo: &str| {
                app.listing.done(&collection, repo);
                app.pages.changed_of(&collection);
            };
            // The profiles may have been reloaded.
            let j = engine.collection(&collection);
            let (configured, gone): (Vec<_>, Vec<_>) =
                repos.iter().partition(|repo| j.as_ref().is_some_and(|j| j.repo(repo).is_some()));
            if let Some(j) = &j {
                let repos: Vec<_> = j.repos().filter(|p| configured.contains(&&p.name)).collect();
                engine.list_repos(j, &repos, |p| listed(&p.name));
            }
            for repo in gone {
                let inventory = Inventory {
                    listed: engine.store.now(),
                    result: Err(Fault::NoLongerConfigured.to_string()),
                    size_outdated: false,
                };
                _ = engine.store.save_inventory(&collection, repo, &inventory);
                listed(repo);
            }
            if let Some(j) = &j {
                engine.notify(j);
            }
        });
    }
}
