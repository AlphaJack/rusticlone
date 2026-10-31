//! Restores and test restores.

use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::Result;
use jiff::{Timestamp, tz::TimeZone};

use super::{Engine, Error, Run, Use, newest, pick};
use crate::request::{RestoreOptions, StepKey, Tested};
use crate::result::Doing;
use crate::{
    profile::RusticProfile,
    result::StepResult,
    rustic::Snapshot,
    state::Event,
    sys::{self, Presence},
};

/// The repository of the run's collection named `name`, else its primary.
fn repo_named<'a>(run: &Run<'a>, name: Option<&str>) -> Result<&'a RusticProfile> {
    let collection = run.collection;
    let name = name.unwrap_or(collection.name());
    let repo = collection.repo(name);
    Ok(repo.ok_or_else(|| Error::NotOfProfile { name: name.to_owned(), collection: collection.title.to_owned() })?)
}

impl Engine {
    /// Reads the tested snapshots of `from`, or of the repository a restore reads, as a restore would, and
    /// reports what restoring them in place would create and replace. Writes nothing.
    pub(super) fn test_restore(&self, run: &Run<'_>, from: Option<&str>, tested: &Tested) -> Result<()> {
        let collection = run.collection;
        self.learn_from_repositories(run);
        let (repo, picks) = match (from, tested) {
            (None, Tested::Newest(source)) => {
                self.newest_repo(run, None, &collection.primary.groups_of(*source), None)?
            }
            (from, Tested::Snapshot(id)) => {
                let repo = repo_named(run, from)?;
                (repo, vec![self.rustic.snapshot(&run.cx, repo, id)?])
            }
            (Some(from), Tested::Newest(source)) => {
                let repo = repo_named(run, Some(from))?;
                (repo, pick(&self.list(run, repo)?, &collection.primary.groups_of(*source), None))
            }
        };
        if picks.is_empty() {
            return Err(Error::NoSnapshotOfThem { repo: repo.name.clone() }.into());
        }
        let _locks = self.lock(run, false, &[(repo, Use::Read)])?;
        for sn in picks.iter().take_while(|_| !run.cx.stopping()) {
            run.activity(Some(Doing::Snapshot { snapshot: sn.id.short().to_owned() }), true);
            let read = self.rustic.dump_size(&run.cx, repo, &sn.id);
            run.idle();
            let bytes = read?;
            // One line per snapshot: what was read, then what a restore would do for each source.
            let mut plans = Vec::new();
            // Stdin was never a file, and a remote source is not here: nothing to compare with.
            let elsewhere = collection.primary.sources.iter().any(|s| !s.in_place() && s.paths() == sn.paths);
            for path in sn.paths.iter().filter(|_| !elsewhere) {
                let created = Created::before(Path::new(path));
                let plan = run.tracked(Some(Doing::Path { path: path.clone() }), |progress| {
                    self.rustic.restore_plan(&run.cx, repo, &sn.id, path, progress)
                });
                created.undo();
                plans.push((path.clone(), plan?));
            }
            let (repo, snapshot) = (repo.name.clone(), sn.id.short().to_owned());
            run.push(StepResult::TestRestored { repo, snapshot, bytes, plans });
        }
        Ok(())
    }

    /// Restores from the repository with the newest snapshots, matched by source paths, not by host.
    pub(super) fn restore_inner(&self, run: &Run<'_>, options: &RestoreOptions) -> Result<()> {
        let collection = run.collection;
        if let Some(id) = &options.snapshot {
            let repo = repo_named(run, options.from.as_deref())?;
            let mut sn = self.rustic.snapshot(&run.cx, repo, id)?;
            if let Some(path) = &options.path {
                sn.paths = vec![path.clone()];
            }
            return self.restore_snapshots(run, repo, &[sn], options).map(drop);
        }
        self.learn_from_repositories(run);
        let mut groups = collection.primary.groups_of(options.source);
        let mut ranked = self.newest_repos(run, options.from.as_deref(), &groups, options.at)?;
        // Made on another computer, where this one's links lead elsewhere: as the profile writes the paths.
        let written = collection.primary.written_groups_of(options.source);
        if ranked.is_empty() && written != groups {
            ranked = self.newest_repos(run, options.from.as_deref(), &written, options.at)?;
            groups = written;
        }
        let mut ranked = ranked.into_iter();
        let (src, picks) =
            ranked.next().ok_or_else(|| Error::NoMatchingSnapshot { repo: collection.title.to_owned() })?;
        if picks.len() < groups.len() {
            run.push(StepResult::SomeSources { found: picks.len(), of: groups.len() });
        }
        // Every source, as it is now, to where it was.
        let all = options.at.is_none() && options.source.is_none() && options.to.is_none();
        // Asked for everything and the primary was lost with the sources: it is re-created from where they come
        // from first, so that they are read from it and a remote copy is downloaded once. If that fails, they
        // come from the copy. A more specific restore leaves the repositories alone.
        let lost = all
            && !options.dry_run
            && options.from.is_none()
            && src.name != collection.name()
            && !run.cx.stopping()
            && self.list(run, collection.primary).is_err();
        let into = |primary: &RusticProfile| {
            if primary.recreatable() {
                return self.copy_into(run, primary, src, &picks);
            }
            let (repo, folder) = (primary.name.clone(), primary.repository.clone().unwrap_or_default());
            Err(Error::FolderAway { repo, folder }.into())
        };
        let (src, picks) = match lost.then(|| into(collection.primary)) {
            Some(Ok(copied)) => {
                let (repo, from) = (collection.name().to_owned(), src.name.clone());
                run.push(StepResult::Recreated { repo, from, snapshots: copied.len() as u64 });
                (collection.primary, copied)
            }
            Some(Err(e)) => {
                let (repo, from, error) = (collection.name().to_owned(), src.name.clone(), format!("{e:#}"));
                run.push(StepResult::NotRecreated { repo, from, error });
                (src, picks)
            }
            None => (src, picks),
        };
        let (src, picks, complete) = self.restore_falling_back(run, (src, picks), ranked, options)?;
        // Every source restored from its newest snapshot: the next backup continues from there.
        let whole = complete && all && picks.len() == groups.len();
        if let Some(oldest) =
            picks.iter().map(|s| s.time).min().filter(|_| whole && !options.dry_run && !run.cx.stopping())
        {
            let (key, note) = (
                StepKey::Snapshot(collection.name().to_owned()),
                StepResult::BackupRestored { from: src.name.clone() },
            );
            self.emit(
                collection.name(),
                &[Event::Attempted {
                    key,
                    started: oldest,
                    at: self.store.now(),
                    result: (&note).into(),
                    by: run.by(run.cx.step()),
                }],
            );
        }
        Ok(())
    }

    /// Restores `picks` from `src`, then, if that failed and no repository was asked for, from each next newest
    /// of `ranked` until one succeeds: a repository whose data is damaged fails partway, and the next one's files
    /// overwrite what it left. Returns the repository and snapshots the restore ended with, and whether it is
    /// complete. Fails as the last attempt did.
    fn restore_falling_back<'a>(
        &self,
        run: &Run<'a>,
        (mut src, mut picks): (&'a RusticProfile, Vec<Snapshot>),
        ranked: impl Iterator<Item = (&'a RusticProfile, Vec<Snapshot>)>,
        options: &RestoreOptions,
    ) -> Result<(&'a RusticProfile, Vec<Snapshot>, bool)> {
        let attempt = |src: &RusticProfile, picks: &[Snapshot]| {
            let failed = run.failures();
            let restored = self.restore_snapshots(run, src, picks, options);
            restored.and_then(|complete| match run.failures() {
                n if n == failed => Ok(complete),
                _ => Err(Error::PartlyRestored { repo: src.name.clone() }.into()),
            })
        };
        let mut done = attempt(src, &picks);
        if options.from.is_none() && !options.dry_run {
            for (next, next_picks) in ranked.filter(|(p, _)| p.name != src.name) {
                let Err(e) = &done else { break };
                // Only what rustic failed to read: another repository cannot help a folder this computer refuses.
                let read = matches!(e.downcast_ref(), Some(Error::NotRestored { .. } | Error::PartlyRestored { .. }));
                if !read || run.cx.stopping() {
                    break;
                }
                let (repo, to, error) = (src.name.clone(), next.name.clone(), format!("{e:#}"));
                run.push(StepResult::FallingBack { repo, to, error });
                (src, picks) = (next, next_picks);
                done = attempt(src, &picks);
            }
        }
        done.map(|complete| (src, picks, complete))
    }

    /// The repository with the newest snapshots of `groups` not after `at`, among `from` or all of the
    /// collection's, and those snapshots.
    fn newest_repo<'a>(
        &self,
        run: &Run<'a>,
        from: Option<&str>,
        groups: &[Vec<String>],
        at: Option<Timestamp>,
    ) -> Result<(&'a RusticProfile, Vec<Snapshot>)> {
        let ranked = self.newest_repos(run, from, groups, at)?;
        let Some((src, picks)) = ranked.into_iter().next() else {
            return Err(Error::NoMatchingSnapshot { repo: run.collection.title.to_owned() }.into());
        };
        if picks.len() < groups.len() {
            run.push(StepResult::SomeSources { found: picks.len(), of: groups.len() });
        }
        Ok((src, picks))
    }

    /// The repositories with snapshots of `groups` not after `at`, among `from` or all of the collection's,
    /// newest first. On a tie the collection's order decides: the primary, then its targets.
    fn newest_repos<'a>(
        &self,
        run: &Run<'a>,
        from: Option<&str>,
        groups: &[Vec<String>],
        at: Option<Timestamp>,
    ) -> Result<Vec<(&'a RusticProfile, Vec<Snapshot>)>> {
        let collection = run.collection;
        let candidates: Vec<&RusticProfile> = match from {
            Some(_) => vec![repo_named(run, from)?],
            None => collection.repos().collect(),
        };
        let (mut ranked, mut unreadable) = (Vec::new(), Vec::new());
        for p in &candidates {
            match self.list(run, p) {
                Ok(snapshots) => {
                    let picks = pick(&snapshots, groups, at);
                    if !picks.is_empty() {
                        ranked.push((*p, picks));
                    }
                }
                Err(e) => {
                    // Lost in a disaster: the next backup re-creates it from a target.
                    if p.name != collection.name() {
                        run.push(StepResult::Unavailable { repo: p.name.clone(), error: format!("{e:#}") });
                    }
                    unreadable.push((*p, format!("{e:#}")));
                }
            }
        }
        // None could be read: its place is likely another computer's, as a disk mounted elsewhere here.
        if unreadable.len() == candidates.len() {
            let repos = unreadable.iter().map(|(p, e)| {
                format!("{} at {}: {}", p.name, p.repository.as_deref().unwrap_or_default(), e.trim_end_matches('.'))
            });
            let files = unreadable.iter().filter_map(|(p, _)| Some(p.files().first()?.display().to_string()));
            let (repos, files) = (repos.collect::<Vec<_>>().join("; "), files.collect::<Vec<_>>().join(", "));
            return Err(Error::Unreadable { collection: collection.title.to_owned(), repos, files }.into());
        }
        // Stable: equal times keep the collection's order.
        ranked.sort_by_key(|(_, picks)| std::cmp::Reverse(newest(picks)));
        Ok(ranked)
    }

    /// Restores each path of each snapshot in place, or below `to`. With `missing`, only paths that are
    /// absent or empty. Reports each path as restored or left as it is.
    #[expect(
        clippy::disallowed_methods,
        reason = "--missing looks at where it restores, which no backup hook prepares"
    )]
    fn restore_snapshots(
        &self,
        run: &Run<'_>,
        repo: &RusticProfile,
        snapshots: &[Snapshot],
        options: &RestoreOptions,
    ) -> Result<bool> {
        let _locks = self.lock(run, false, &[(repo, Use::Read)])?;
        let mut complete = true;
        for sn in snapshots {
            let source = run.collection.primary.sources.iter().find(|s| s.paths() == sn.paths);
            let unplaced = source.filter(|_| options.to.is_none()).and_then(|s| {
                let source = s.sources.join(", ");
                // A folder of another system has no place on this one.
                let foreign = s.sources.iter().any(|p| sys::foreign_path(p));
                if s.in_place() && !foreign {
                    s.hooked.then_some(StepResult::Hooked { source })
                } else {
                    Some(StepResult::NoPlace { source })
                }
            });
            if let Some(result) = unplaced {
                complete = false;
                run.push(result);
                continue;
            }
            for path in sn.paths.iter().take_while(|_| !run.cx.stopping()) {
                // A remote source is the snapshot's root, `""`: named by its source instead.
                let shown = source.filter(|_| path.is_empty()).map_or_else(|| path.clone(), |s| s.sources.join(", "));
                // Below `to`, a path of the snapshot keeps its name, and sources their full path, so that
                // two sources cannot collide.
                let below = |root: &Path| match (&options.path, Path::new(path).file_name()) {
                    (Some(_), Some(name)) => root.join(name),
                    _ => root.join(below_root(path)),
                };
                let dest = options.to.as_deref().map_or_else(|| PathBuf::from(path), below);
                match if options.missing { sys::presence(&dest) } else { Presence::Missing } {
                    Presence::Present => {
                        run.push(StepResult::LeftAsItIs { path: path.clone() });
                        continue;
                    }
                    Presence::Unmounted => {
                        complete = false;
                        run.push(StepResult::Unmounted { path: path.clone() });
                        continue;
                    }
                    Presence::Missing => {}
                }
                let to = (dest != Path::new(path)).then_some(dest.clone());
                if self.store.is_dry_run() || options.dry_run {
                    let (files, bytes) =
                        sn.summary.as_ref().map_or((0, 0), |s| (s.total_files_processed, s.total_bytes_processed));
                    let (repo, snapshot) = (repo.name.clone(), sn.id.short().to_owned());
                    run.push(StepResult::WouldRestore { path: shown, repo, snapshot, files, bytes, to });
                    continue;
                }
                #[expect(clippy::disallowed_methods, reason = "creates the restore destination")]
                fs::create_dir_all(dest.parent().unwrap_or(Path::new("/")))?;
                run.tracked(Some(Doing::Path { path: path.clone() }), |progress| {
                    self.rustic.restore(&run.cx, repo, &sn.id, path, &dest, progress)
                })
                .map_err(|e| Error::NotRestored { path: shown.clone(), source: e.into() })?;
                run.push(StepResult::Restored {
                    path: shown,
                    repo: repo.name.clone(),
                    snapshot: sn.id.short().to_owned(),
                    time: sn.time,
                    to,
                });
            }
        }
        Ok(complete)
    }
}

pub(crate) fn parse_at(s: &str) -> Result<Timestamp, Error> {
    if let Ok(ts) = s.parse::<Timestamp>() {
        return Ok(ts);
    }
    let dt = match s.trim().parse::<jiff::civil::Date>() {
        // A bare date means the end of that day.
        Ok(date) => date.to_datetime(jiff::civil::Time::MAX),
        Err(_) => s
            .replace(' ', "T")
            .parse::<jiff::civil::DateTime>()
            .map_err(|source| Error::NoTime { time: s.to_owned(), source })?,
    };
    (TimeZone::system().to_ambiguous_zoned(dt).compatible())
        .map(|z| z.timestamp())
        .map_err(|source| Error::NoTime { time: s.to_owned(), source })
}

/// `/home/me/docs` becomes `home/me/docs`, `C:\Users\me` becomes `C/Users/me`, on any system.
fn below_root(path: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for c in Path::new(crate::rustic::tree_path(path).as_ref()).components() {
        match c {
            Component::Normal(part) => out.push(part),
            Component::Prefix(prefix) => {
                out.extend(prefix.as_os_str().to_string_lossy().split(['\\', '/', ':', '?']).filter(|p| !p.is_empty()));
            }
            Component::RootDir | Component::CurDir | Component::ParentDir => {}
        }
    }
    out
}

/// The directories missing above and at a path, which rustic's dry-run restore creates as its destination
/// although it writes nothing else.
struct Created<'a> {
    path: &'a Path,
    /// The nearest that exists.
    existing: Option<&'a Path>,
}

impl<'a> Created<'a> {
    #[expect(clippy::disallowed_methods, reason = "finds what a test restore must leave as it was")]
    fn before(path: &'a Path) -> Self {
        Self { path, existing: path.ancestors().find(|a| a.exists()) }
    }

    /// Removes them, deepest first, as long as they are empty.
    fn undo(&self) {
        let created = self.path.ancestors().take_while(|a| Some(*a) != self.existing);
        for dir in created {
            if fs::remove_dir(dir).is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restored_below_a_root() {
        assert_eq!(below_root("/home/me/docs"), PathBuf::from("home/me/docs"));
        assert_eq!(below_root("relative/x"), PathBuf::from("relative/x"));
    }
}
