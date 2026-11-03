//! Watching profiles and the state for changes.

#![expect(clippy::disallowed_methods, reason = "watches profiles and the state")]

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher, event::ModifyKind};
use parking_lot::Mutex;
use tokio::sync::Notify;

use super::App;
use super::Logged;
use crate::{log, profile, state::Changes};

/// Asks the server to read the profiles again, with the files that changed since it last did.
#[derive(Default)]
pub(super) struct Reload {
    pub(super) asked: Notify,
    changed: Mutex<BTreeSet<PathBuf>>,
}

impl Reload {
    fn changed(&self, path: &Path) {
        self.changed.lock().insert(path.to_owned());
        self.asked.notify_one();
    }

    /// The files that changed since the last call.
    pub(super) fn take(&self) -> super::Changed {
        super::Changed(std::mem::take(&mut *self.changed.lock()).iter().map(|p| p.display().to_string()).collect())
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
            match changed(path, listing_changed, &dirs, &runs) {
                File::Profile => reload.changed(path),
                // The runs of this process tell their pages as they log.
                File::RunLog { collection, id } if !app.runs.of(collection).iter().any(|l| Some(l.id) == id) => {
                    app.pages.changed_of(collection);
                }
                File::RunLog { .. } | File::Other => {}
                File::State => {
                    let changes = app.engine().store.changes();
                    let mut seen = seen.lock();
                    let (pages, due) = shown(&seen, &changes);
                    for collection in pages {
                        match collection {
                            Some(collection) => app.pages.changed_of(collection),
                            None => app.pages.changed(),
                        }
                    }
                    if due {
                        app.wake.notify_one();
                    }
                    *seen = changes;
                }
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

/// What a file that changed is to the server.
#[derive(Debug, PartialEq)]
enum File<'a> {
    /// A profile, to read again.
    Profile,
    /// The log of a run, `runs/<collection>/<id>.log`, with its id if the name is one.
    RunLog {
        collection: &'a str,
        id: Option<i64>,
    },
    /// The state.
    State,
    Other,
}

/// What `path` is, in the profile folders `dirs` or under the state's `runs` folder. A file in a profile folder is
/// a profile when it is `.toml`, or whenever the folder's listing changed.
fn changed<'a>(path: &'a Path, listing_changed: bool, dirs: &[PathBuf], runs: &Path) -> File<'a> {
    if dirs.iter().any(|d| path.parent() == Some(d))
        && (listing_changed || path.extension().is_some_and(|e| e == "toml"))
    {
        return File::Profile;
    }
    if let Ok(log) = path.strip_prefix(runs) {
        let mut parts = log.iter().filter_map(|p| p.to_str());
        let (Some(collection), id) = (parts.next(), parts.next().and_then(|f| f.strip_suffix(".log"))) else {
            return File::Other;
        };
        return File::RunLog { collection, id: id.and_then(|id| id.parse().ok()) };
    }
    if path.file_name().is_some_and(|f| f == "state.redb") { File::State } else { File::Other }
}

/// The pages that a change of the state from `seen` to `now` shows on: the collections whose writes changed, and
/// `None` for every page. Whether what is due may have changed too: it depends on the ledger, and on the runs that are
/// not over.
fn shown<'a>(seen: &Changes, now: &'a Changes) -> (Vec<Option<&'a str>>, bool) {
    let pages = now.writes.iter().filter(|(c, n)| seen.writes.get(*c) != Some(n)).map(|(c, _)| c.as_deref()).collect();
    (pages, now.writes.get(&None) != seen.writes.get(&None) || now.unfinished != seen.unfinished)
}
