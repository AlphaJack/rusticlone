//! The collections that profiles define, loaded into the engine, and what the command line names of them.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

use anyhow::Result;
use jiff::tz::TimeZone;

use super::{
    Collection, Engine, Error,
    collection::{continues, title},
};
use crate::request::{Request, RestoreOptions, RunKind, Selection};
use crate::result::Unusable;
use crate::{
    live::Runs,
    profile::{self, RusticProfile},
    rustic::Rustic,
    state::Store,
};

/// The collections and repositories a command names, by profile. Nothing named: every collection.
#[derive(Clone, Default)]
pub(crate) struct Named {
    pub collections: Vec<String>,
    /// A collection's primary, or a copy target.
    pub repos: Vec<String>,
}

impl Named {
    fn is_empty(&self) -> bool {
        self.collections.is_empty() && self.repos.is_empty()
    }
}

impl Engine {
    pub(crate) fn new(rustic: Rustic, store: Store, runs: Arc<Runs>, mut named: Named) -> Result<Self> {
        let all = profile::profile_files()?;
        let files = if named.is_empty() {
            all.clone()
        } else {
            let mut files = Vec::new();
            for n in named.collections.iter().chain(&named.repos) {
                match profile::locate(n) {
                    Ok(dir) => files.push((n.clone(), dir)),
                    Err(e) if !named.collections.contains(n) => return Err(e),
                    // A collection named by its title is among the profiles that continue it. Named by
                    // neither, every profile is read, so that the error lists the collections there are.
                    Err(_) => {
                        let titled: Vec<_> = all.iter().filter(|(f, _)| continues(f, n)).cloned().collect();
                        files.extend(if titled.is_empty() { all.clone() } else { titled });
                    }
                }
            }
            files
        };
        let load = |name: &str, dir: &PathBuf| -> Result<RusticProfile> {
            let p = profile::load(name, dir, &rustic.config(name, dir)?)?;
            resolve_hooked(&store, &p);
            Ok(p)
        };
        let loaded = super::each(&files, |(name, dir)| load(name, dir));
        let (mut rustic_profiles, mut broken, mut skipped) = (BTreeMap::new(), Vec::new(), BTreeSet::new());
        let cwd = std::env::current_dir().ok();
        for ((name, dir), p) in files.into_iter().zip(loaded) {
            match p {
                Ok(p) => _ = rustic_profiles.insert(name, p),
                // Other tools keep their own TOML files in the working directory.
                Err(e)
                    if Some(&dir) == cwd.as_ref()
                        && matches!(e.downcast_ref(), Some(crate::rustic::Error::InvalidProfile { .. }))
                        && !named.collections.iter().chain(&named.repos).any(|n| *n == name) =>
                {
                    skipped.insert(name);
                }
                Err(e) => broken.push((name, format!("{e:#}"))),
            }
        }
        // A repository was named: add the collections that copy to it, wherever they are.
        if named.repos.iter().any(|n| rustic_profiles.get(n).is_some_and(|p| !p.is_backup_profile())) {
            for (name, dir) in profile::profile_files()? {
                if !rustic_profiles.contains_key(&name)
                    && let Ok(p) = load(&name, &dir)
                    && p.is_backup_profile()
                    && p.targets.iter().any(|t| named.repos.contains(t))
                {
                    rustic_profiles.insert(name, p);
                }
            }
        }
        // Rustic resolves copy targets itself, possibly outside the scanned directories.
        let wanted: BTreeSet<(String, PathBuf)> = rustic_profiles
            .values()
            .filter(|p| p.is_backup_profile())
            .flat_map(|p| p.targets.iter().map(|t| (t.clone(), p.dir.clone())))
            .filter(|(t, _)| !rustic_profiles.contains_key(t))
            .collect();
        for (name, dir) in wanted {
            if let Ok(p) = load(&name, &dir)
                && p.repository.is_some()
            {
                rustic_profiles.insert(name, p);
            }
        }
        let profiles = all.into_iter().map(|(name, _)| name).filter(|n| !skipped.contains(n)).collect();
        let mut engine = Self::with_profiles(rustic, store, runs, rustic_profiles, broken, profiles, Named::default());
        for n in &mut named.collections {
            if let Some(j) = engine.collections().into_iter().find(|j| j.title == n) {
                *n = j.name().to_owned();
            }
        }
        engine.named = named;
        Ok(engine)
    }

    pub(crate) fn with_profiles(
        rustic: Rustic,
        store: Store,
        runs: Arc<Runs>,
        rustic_profiles: BTreeMap<String, RusticProfile>,
        broken: Vec<(String, String)>,
        profiles: Vec<String>,
        named: Named,
    ) -> Self {
        Self {
            rustic,
            store,
            runs,
            rustic_profiles,
            broken,
            profiles,
            named,
            tz: TimeZone::system(),
            #[cfg(feature = "server")]
            conditions: &crate::sys::ThisComputer,
        }
    }

    pub(crate) fn collections(&self) -> Vec<Collection<'_>> {
        self.every_collection().filter(|c| self.named.is_empty() || self.selected(c.primary)).collect()
    }

    /// Every collection, whether named or not.
    pub(crate) fn every_collection(&self) -> impl Iterator<Item = Collection<'_>> {
        self.rustic_profiles.values().filter(|p| p.is_backup_profile()).map(|p| {
            let (targets, missing): (Vec<_>, _) = p.targets.iter().partition(|t| self.rustic_profiles.contains_key(*t));
            Collection {
                primary: p,
                title: title(&p.name, &p.targets, &self.profiles),
                targets: targets.into_iter().map(|t| &self.rustic_profiles[t]).collect(),
                missing_targets: missing.into_iter().cloned().collect(),
            }
        })
    }

    pub(super) fn unusable(&self, target: &str) -> Unusable {
        Unusable { target: target.to_owned(), broken: self.broken.iter().any(|(name, _)| name == target) }
    }

    pub(crate) fn collection(&self, name: &str) -> Option<Collection<'_>> {
        self.collections().into_iter().find(|j| j.name() == name)
    }

    /// What the collection `name` is called, or `name` if it is gone.
    #[cfg(feature = "server")]
    pub(crate) fn title(&self, name: &str) -> String {
        self.collection(name).map_or_else(|| name.to_owned(), |j| j.title.to_owned())
    }

    /// The collections that `selection` names, each with the repository it names.
    pub(crate) fn named(&self, selection: &Selection) -> Result<Vec<(Collection<'_>, Option<&RusticProfile>)>> {
        let collections = match selection.collection() {
            None => self.collections(),
            Some(name) => vec![self.collection(name).ok_or_else(|| self.no_collection(name))?],
        };
        collections
            .into_iter()
            .map(|j| {
                let repo = match selection {
                    Selection::Repo(_, repo) => Some(j.repo_named(repo)?),
                    Selection::All | Selection::Collection(_) => None,
                };
                Ok((j, repo))
            })
            .collect()
    }

    fn no_collection(&self, name: &str) -> Error {
        let known: Vec<String> = self.every_collection().map(|c| c.name().to_owned()).collect();
        match known.is_empty() {
            true => Error::NoCollections { dirs: profile::dirs_searched() },
            false => Error::NoCollection { name: name.to_owned(), known: known.join(", ") },
        }
    }

    /// Named on the command line, as a collection or by one of its repositories.
    fn selected(&self, collection: &RusticProfile) -> bool {
        let Named { collections, repos } = &self.named;
        collections.contains(&collection.name)
            || std::iter::once(&collection.name).chain(&collection.targets).any(|n| repos.contains(n))
    }

    /// What the command line names: each collection, and each repository in every collection that has it.
    /// A repository of a named collection narrows that collection to it, and leaves out the other collections
    /// that copy to it. Every collection if nothing is named.
    pub(crate) fn selections(&self) -> Result<Vec<Selection>> {
        if self.named.is_empty() {
            return Ok(vec![Selection::All]);
        }
        if let Some(name) = self.named.collections.iter().find(|n| self.collection(n).is_none()) {
            return Err(self.no_collection(name).into());
        }
        let collections = self.collections();
        let has = |j: &Collection<'_>, repo: &String| j.repos().any(|r| r.name == *repo);
        let named = |j: &&Collection<'_>| self.named.collections.iter().any(|c| c == j.name());
        let narrowed =
            |c: &String| collections.iter().any(|j| j.name() == c && self.named.repos.iter().any(|r| has(j, r)));
        let mut out: Vec<Selection> =
            self.named.collections.iter().filter(|c| !narrowed(c)).map(|c| Selection::Collection(c.clone())).collect();
        for repo in &self.named.repos {
            let narrows = collections.iter().filter(named).any(|j| has(j, repo));
            let of = collections.iter().filter(|j| has(j, repo) && (!narrows || named(j)));
            let found: Vec<Selection> = of.map(|j| Selection::Repo(j.name().to_owned(), repo.clone())).collect();
            if found.is_empty() {
                return Err(Error::NoRepository { name: repo.clone() }.into());
            }
            out.extend(found);
        }
        Ok(out)
    }

    /// The runs of `kind` for each collection that `selection` names, in collection order. For every collection, only
    /// those that can copy.
    pub(crate) fn requests(
        &self,
        selection: &Selection,
        kind: RunKind,
        options: &RestoreOptions,
    ) -> Result<Vec<(String, Request)>> {
        let mut out = Vec::new();
        for (collection, repo) in self.named(selection)? {
            match collection.requests(kind, repo, options) {
                Err(e) if *selection == Selection::All && kind == RunKind::Copy => {
                    crate::log::event(Some(collection.name()), &crate::log::Logged::NotCopied { error: e.to_string() });
                }
                requests => out.extend(requests?.into_iter().map(|r| (collection.name().to_owned(), r))),
            }
        }
        Ok(out)
    }
}

/// The paths that rustic recorded for sources that hooks create, as learned before.
fn resolve_hooked(store: &Store, p: &RusticProfile) {
    for s in p.sources.iter().filter(|s| s.unknown()) {
        if let Some(paths) = store.hooked_paths(&p.name, &s.sources) {
            s.learn(paths);
        }
    }
}
