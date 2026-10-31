//! Watching profiles and the state for changes.

#![expect(clippy::disallowed_methods, reason = "watches profiles and the state")]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher, event::ModifyKind};
use parking_lot::Mutex;
use tokio::sync::Notify;

use super::App;
use super::Logged;
use crate::{log, profile};

/// A profile file as last read: when it was modified, and its size.
type Seen = BTreeMap<PathBuf, (Option<SystemTime>, u64)>;

/// Asks the server to read the profiles again, with the files that changed since it last did.
pub(super) struct Reload {
    pub(super) asked: Notify,
    changed: Mutex<BTreeSet<PathBuf>>,
    /// The profile files at the last reload. The watcher's events only say that something changed: macOS
    /// reports some files written at once several times, and others not at all.
    seen: Mutex<Seen>,
}

impl Default for Reload {
    fn default() -> Self {
        Self { asked: Notify::new(), changed: Mutex::default(), seen: Mutex::new(profile_files()) }
    }
}

/// Every `.toml` file of the profile folders, also those another folder's profile of the same name hides.
fn profile_files() -> Seen {
    let dirs = profile::profile_dirs().unwrap_or_default();
    let entries = dirs.iter().flat_map(|d| fs::read_dir(d).into_iter().flatten().flatten());
    let files = entries.filter(|e| e.path().extension().is_some_and(|x| x == "toml"));
    files.filter_map(|e| e.metadata().ok().map(|m| (e.path(), (m.modified().ok(), m.len())))).collect()
}

impl Reload {
    fn changed(&self, path: &Path) {
        self.changed.lock().insert(path.to_owned());
        self.asked.notify_one();
    }

    /// The profile files added, changed or removed since the last call, else those the watcher reported.
    pub(super) fn take(&self) -> super::Changed {
        let reported = std::mem::take(&mut *self.changed.lock());
        let now = profile_files();
        let old = std::mem::replace(&mut *self.seen.lock(), now.clone());
        let paths: BTreeSet<&PathBuf> = old.keys().chain(now.keys()).collect();
        let differ: BTreeSet<PathBuf> = paths.into_iter().filter(|p| old.get(*p) != now.get(*p)).cloned().collect();
        let changed = if differ.is_empty() { reported } else { differ };
        super::Changed(changed.iter().map(|p| p.display().to_string()).collect())
    }
}

/// Edited profiles are read again, and state changes, also from other processes, reach
/// the web UI at once. Without a watcher (no inotify watches left, for example), `systemctl reload`
/// still reads the profiles again.
pub(super) fn watch_files(state: &Path, app: &Arc<App>, reload: &Arc<Reload>) -> Option<RecommendedWatcher> {
    let profile_dirs = profile::profile_dirs().unwrap_or_default();
    let (dirs, app, reload, runs) = (profile_dirs.clone(), app.clone(), reload.clone(), state.join("runs"));
    // The change counts last seen.
    let seen = Mutex::new(app.engine().store.changes());
    let handle = move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        // macOS reports a profile renamed to `Docs.toml.old` as the creation of that file only.
        let listing_changed = !matches!(event.kind, EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Metadata(_)));
        for path in &event.paths {
            if dirs.iter().any(|d| path.parent() == Some(d))
                && (listing_changed || path.extension().is_some_and(|e| e == "toml"))
            {
                reload.changed(path);
            } else if let Ok(log) = path.strip_prefix(&runs) {
                // `runs/<collection>/<id>.log`. The runs of this process tell their pages as they log.
                let mut parts = log.iter().filter_map(|p| p.to_str());
                let (Some(collection), id) = (parts.next(), parts.next().and_then(|f| f.strip_suffix(".log"))) else {
                    continue;
                };
                let id = id.and_then(|id| id.parse::<i64>().ok());
                if !app.runs.of(collection).iter().any(|l| Some(l.id) == id) {
                    app.pages.changed_of(collection);
                }
            } else if path.file_name().is_some_and(|f| f == "state.redb") {
                let changes = app.engine().store.changes();
                let mut seen = seen.lock();
                for (collection, _) in changes.writes.iter().filter(|(c, n)| seen.writes.get(*c) != Some(n)) {
                    match collection {
                        Some(collection) => app.pages.changed_of(collection),
                        None => app.pages.changed(),
                    }
                }
                // What is due depends on the ledger, and on the runs that are not over.
                if changes.writes.get(&None) != seen.writes.get(&None) || changes.unfinished != seen.unfinished {
                    app.wake.notify_one();
                }
                *seen = changes;
            }
        }
    };
    let mut watcher = match notify::recommended_watcher(handle) {
        Ok(w) => w,
        Err(e) => {
            log::event(None, &Logged::NotWatching { error: e.to_string() });
            return None;
        }
    };
    // Create the user's profile directory, so that a first profile saved there is seen.
    if let Some(dir) = profile_dirs.first()
        && let Err(e) = fs::create_dir_all(dir)
    {
        log::event(None, &Logged::DirNotCreated { dir: dir.display().to_string(), error: e.to_string() });
    }
    for dir in profile_dirs.iter().filter(|d| d.is_dir()) {
        if let Err(e) = watcher.watch(dir, RecursiveMode::NonRecursive) {
            log::event(None, &Logged::NotWatchingDir { dir: dir.display().to_string(), error: e.to_string() });
        }
    }
    _ = watcher.watch(state, RecursiveMode::Recursive);
    Some(watcher)
}
