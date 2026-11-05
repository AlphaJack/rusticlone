//! Watching profiles and the state for changes.

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
#[expect(clippy::disallowed_methods, reason = "watches profiles and the state")]
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
        let here = |collection: &str| app.runs.of(collection).iter().map(|l| l.id).collect();
        for path in &event.paths {
            match react(path, event.kind, &dirs, &runs, here) {
                React::Reload => reload.changed(path),
                React::Page(collection) => app.pages.changed_of(collection),
                React::Nothing => {}
                React::State => {
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

/// What the server does about a file that changed.
#[derive(Debug, PartialEq)]
enum React<'a> {
    /// Reads the profiles again.
    Reload,
    /// Shows a run's log that another process writes, on its collection's page.
    Page(&'a str),
    /// Reads the state again.
    State,
    Nothing,
}

/// What a change of `kind` to `path` asks of the server. `here`: the ids of this process's runs of a collection,
/// which tell their pages as they log.
fn react<'a>(
    path: &'a Path,
    kind: EventKind,
    dirs: &[PathBuf],
    runs: &Path,
    here: impl Fn(&str) -> Vec<i64>,
) -> React<'a> {
    // macOS reports a profile renamed to `Docs.toml.old` as the creation of that file only.
    let listing_changed = !matches!(kind, EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Metadata(_)));
    match changed(path, listing_changed, dirs, runs) {
        File::Profile => React::Reload,
        File::RunLog { collection, id } if !here(collection).iter().any(|&run| Some(run) == id) => {
            React::Page(collection)
        }
        File::RunLog { .. } | File::Other => React::Nothing,
        File::State => React::State,
    }
}

/// The pages that a change of the state from `seen` to `now` shows on: the collections whose writes changed, and
/// `None` for every page. Whether what is due may have changed too: it depends on the ledger, and on the runs that are
/// not over.
fn shown<'a>(seen: &Changes, now: &'a Changes) -> (Vec<Option<&'a str>>, bool) {
    let pages = now.writes.iter().filter(|(c, n)| seen.writes.get(*c) != Some(n)).map(|(c, _)| c.as_deref()).collect();
    (pages, now.writes.get(&None) != seen.writes.get(&None) || now.unfinished != seen.unfinished)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;

    /// A profile is a `.toml` file in a profile folder, or any file there once the folder's listing changed. A
    /// run's log is named by its collection and id.
    #[test]
    fn a_changed_file_is_a_profile_a_run_s_log_or_the_state() {
        let (profiles, state) = (Path::new("/profiles"), Path::new("/state"));
        let (dirs, runs) = ([profiles.to_owned()], state.join("runs"));
        let (toml, old, nested) =
            (profiles.join("Docs.toml"), profiles.join("Docs.toml.old"), profiles.join("old").join("Docs.toml"));
        assert_eq!(changed(&toml, false, &dirs, &runs), File::Profile);
        assert_eq!(changed(&old, false, &dirs, &runs), File::Other, "only its data changed");
        assert_eq!(changed(&old, true, &dirs, &runs), File::Profile, "renamed away");
        assert_eq!(changed(&nested, true, &dirs, &runs), File::Other, "below a profile folder");
        let (log, notes) = (runs.join("Docs").join("17.log"), runs.join("Docs").join("notes.log"));
        assert_eq!(changed(&log, false, &dirs, &runs), File::RunLog { collection: "Docs", id: Some(17) });
        assert_eq!(changed(&notes, false, &dirs, &runs), File::RunLog { collection: "Docs", id: None });
        assert_eq!(changed(&runs, false, &dirs, &runs), File::Other);
        let (redb, locks) = (state.join("state.redb"), state.join("locks"));
        assert_eq!(changed(&redb, false, &dirs, &runs), File::State);
        assert_eq!(changed(&locks, false, &dirs, &runs), File::Other);
    }

    /// A profile's data changed: it is read again, as when one appears. A file renamed away from a profile changed the
    /// listing only then. The log of a run in another process shows on its page. One of this process's tells it itself.
    #[test]
    fn a_change_asks_what_its_file_needs() {
        use notify::event::{CreateKind, DataChange};
        let (profiles, runs) = (Path::new("/profiles"), Path::new("/state").join("runs"));
        let dirs = [profiles.to_owned()];
        let data = EventKind::Modify(ModifyKind::Data(DataChange::Content));
        let create = EventKind::Create(CreateKind::File);
        let here = |collection: &str| if collection == "Docs" { vec![17] } else { Vec::new() };
        let (toml, old, state) =
            (profiles.join("Docs.toml"), profiles.join("Docs.toml.old"), Path::new("/state").join("state.redb"));
        let (own, other, pics) =
            (runs.join("Docs").join("17.log"), runs.join("Docs").join("18.log"), runs.join("Pics").join("17.log"));
        assert_eq!(react(&toml, data, &dirs, &runs, here), React::Reload);
        assert_eq!(react(&old, data, &dirs, &runs, here), React::Nothing);
        assert_eq!(react(&old, create, &dirs, &runs, here), React::Reload);
        assert_eq!(react(&own, data, &dirs, &runs, here), React::Nothing);
        assert_eq!(react(&other, data, &dirs, &runs, here), React::Page("Docs"));
        assert_eq!(react(&pics, data, &dirs, &runs, here), React::Page("Pics"));
        assert_eq!(react(&state, data, &dirs, &runs, here), React::State);
    }

    /// A change of the state shows on the pages of the collections it wrote to, and on every page when it wrote to
    /// all. What is due may have changed when it wrote to all, or when a run started or ended.
    #[test]
    fn a_change_of_the_state_shows_where_it_wrote() {
        let changes = |writes: &[(Option<&str>, u64)], unfinished: &[(&str, i64)]| Changes {
            writes: writes.iter().map(|&(c, n)| (c.map(Into::into), n)).collect::<BTreeMap<_, _>>(),
            unfinished: unfinished.iter().map(|&(c, id)| (c.into(), id)).collect::<BTreeSet<_>>(),
        };
        let seen = changes(&[(None, 1), (Some("Docs"), 1)], &[]);
        assert_eq!(shown(&seen, &changes(&[(None, 1), (Some("Docs"), 2)], &[])), (vec![Some("Docs")], false));
        let written = changes(&[(None, 2), (Some("Docs"), 1), (Some("Pics"), 1)], &[]);
        assert_eq!(shown(&seen, &written), (vec![None, Some("Pics")], true));
        assert_eq!(shown(&seen, &changes(&[(None, 1), (Some("Docs"), 1)], &[("Docs", 7)])), (vec![], true));
        assert_eq!(shown(&seen, &seen), (vec![], false));
    }
}
