//! Backups into the primary, and re-creating a lost repository from another one.

use std::collections::BTreeSet;

use anyhow::Result;

use super::{Engine, Error, Run, Use, newest, shrunk};
use crate::request::StepKey;
use crate::result::Doing;
use crate::{
    profile::{RusticProfile, Sources},
    request::Trigger,
    result::StepResult,
    rustic::{self, Snapshot, SnapshotId},
    state::Event,
};

/// Unreadable items named in a backup's warning. The warning counts all of them.
const SKIPPED_SHOWN: usize = 5;

impl Engine {
    pub(super) fn backup(&self, run: &Run<'_>) {
        let collection = run.collection;
        let (Some(key), started) = (run.key(), self.store.now()) else { return };
        // Not a failure. The backup waits for its next scheduled time; otherwise it would be due, and held
        // back, again at once.
        if let Some(why) = self.held_back(run) {
            run.push(why.clone());
            let (at, by) = (self.store.now(), run.by(run.cx.step()));
            let held = Event::Attempted { key: key.clone(), started, at, result: (&why).into(), by };
            return self.emit(collection.name(), &[held, Event::Warned { key, warnings: vec![(&why).into()] }]);
        }
        let result = self.backup_inner(run);
        // Also after a failure: a backup of several sources may have saved some.
        let targets = collection.targets.iter().map(|t| t.name.clone()).collect();
        self.emit(collection.name(), &[Event::Pending { collection: collection.name().to_owned(), targets }]);
        // Listed for the web UI and the copies that follow, then measured.
        let listed = result.as_ref().ok().and_then(|_| self.list(run, collection.primary).ok());
        if listed.is_some() {
            self.measure(run, collection.primary);
        }
        let skipped = result.as_ref().map(|(_, warnings)| warnings.clone()).unwrap_or_default();
        let listed = listed.unwrap_or_default();
        let shrunk = shrunk(&listed).into_iter().map(|(newest, held)| {
            let size = |s: &Snapshot| s.summary.as_ref().map_or(0, |s| s.total_bytes_processed);
            let held = held.iter().max_by_key(|s| s.time).map(|s| size(s));
            StepResult::Shrunk { paths: newest.paths.clone(), holds: size(newest), held }
        });
        let warnings: Vec<StepResult> = skipped.into_iter().chain(shrunk).collect();
        self.step(run, started, result.map(|(note, _)| note));
        // Kept until a backup reads everything.
        if !warnings.is_empty() {
            warnings.iter().cloned().for_each(|w| run.push(w));
            self.emit(collection.name(), &[Event::Warned { key, warnings: warnings.iter().map(Into::into).collect() }]);
        }
    }

    /// Why a scheduled backup must wait: the collection was never backed up on this computer, the repository
    /// has this computer's snapshots of a source, and rustic finds that source absent or empty. The computer
    /// was likely reinstalled: a backup now would become the newest snapshot, which a restore takes, and it
    /// would be empty. Rustic decides, in a dry run, because the backup's hooks may mount or fill a source.
    /// If the sources are present (only the state was lost), the backup runs.
    fn held_back(&self, run: &Run<'_>) -> Option<StepResult> {
        let collection = run.collection;
        let scheduled = run.live.as_ref().is_some_and(|l| l.trigger() == Trigger::Schedule);
        if !scheduled || self.store.load().succeeded(&StepKey::Snapshot(collection.name().to_owned())) {
            return None;
        }
        let listed = self.list(run, collection.primary).ok()?;
        let here = |src: &&Sources| {
            listed.iter().any(|s| src.host == s.hostname && rustic::source_key(&src.paths()) == s.source())
        };
        let kept: Vec<&Sources> = collection.primary.sources.iter().filter(here).collect();
        if kept.is_empty() {
            return None;
        }
        let plan = {
            let _locks = self.lock(run, false, &[(collection.primary, Use::Read)]).ok()?;
            self.rustic.backup_plan(&run.cx, collection.primary).ok()?
        };
        // Rustic names a failed source by its paths, joined with commas.
        let failed = |src: &Sources| {
            let named = |path: &String| path.split(',').any(|p| src.sources.iter().any(|s| s == p));
            plan.not_backed_up.iter().any(|(path, _)| named(path))
        };
        let empty = |src: &Sources| {
            let key = rustic::source_key(&src.paths());
            plan.snapshots.iter().any(|s| s.files == 0 && rustic::source_key(&s.paths) == key)
        };
        let paths: Vec<String> = kept.into_iter().filter(|s| failed(s) || empty(s)).flat_map(Sources::paths).collect();
        (!paths.is_empty()).then_some(StepResult::HeldBack { repo: collection.name().to_owned(), paths })
    }

    /// Re-creates the unlistable primary with every snapshot of `from`, else of the copy target with the
    /// newest snapshots. Without copy targets, re-creates it empty.
    pub(super) fn copy_back(&self, run: &Run<'_>, from: Option<&RusticProfile>) {
        let collection = run.collection;
        // Nothing to copy back from: the backups fill it.
        if collection.targets.is_empty() && collection.missing_targets.is_empty() {
            let created = self
                .lock(run, false, &[(collection.primary, Use::Add)])
                .and_then(|_locks| self.rustic.init(&run.cx, collection.primary));
            self.created(run, collection.primary);
            return match created {
                Ok(()) => run.push(StepResult::PrimaryRecreatedEmpty { repo: collection.name().to_owned() }),
                Err(e) => run.push(run.failed(&e)),
            };
        }
        let (src, snapshots) = match from {
            Some(src) => match self.list(run, src) {
                Ok(snapshots) => (src, snapshots),
                Err(e) => return run.push(run.failed(&e.context(Error::NotListed { repo: src.name.clone() }))),
            },
            None => {
                let listed = collection.targets.iter().filter_map(|t| self.list(run, t).ok().map(|s| (*t, s)));
                let Some(newest) = listed.max_by_key(|(_, s)| newest(s)) else {
                    return run.push(StepResult::NoTargetListed);
                };
                newest
            }
        };
        match self.copy_into(run, collection.primary, src, &snapshots) {
            Ok(copied) => {
                run.push(StepResult::Recreated {
                    repo: collection.name().to_owned(),
                    from: src.name.clone(),
                    snapshots: copied.len() as u64,
                });
            }
            Err(e) => run.push(run.failed(&e.context(Error::NotCopiedFrom { repo: src.name.clone() }))),
        }
    }

    /// Re-creates a target with all snapshots of `from`, else empty. The primary's newer snapshots follow
    /// with the next copy.
    pub(super) fn recreate_target(&self, run: &Run<'_>, t: &RusticProfile, from: Option<&RusticProfile>) {
        let created = match from {
            Some(src) => self.list(run, src).and_then(|held| self.copy_into(run, t, src, &held)).map(|copied| {
                StepResult::Recreated { repo: t.name.clone(), from: src.name.clone(), snapshots: copied.len() as u64 }
            }),
            None => self
                .lock(run, false, &[(run.collection.primary, Use::Read), (t, Use::Add)])
                .and_then(|_locks| self.create_target(run, t))
                .map(|()| StepResult::TargetRecreated { target: t.name.clone() }),
        };
        run.push(created.unwrap_or_else(|e| run.failed(&e)));
    }

    /// The backup's result, and the items it could not read.
    fn backup_inner(&self, run: &Run<'_>) -> Result<(StepResult, Vec<StepResult>)> {
        let collection = run.collection;
        // What rustic would save, its hooks included, into a repository that must exist already.
        if self.store.is_dry_run() {
            if self.list(run, collection.primary).is_err() {
                return Ok((StepResult::WouldCreatePrimary { repo: collection.name().to_owned() }, Vec::new()));
            }
            let _locks = self.lock(run, false, &[(collection.primary, Use::Read)])?;
            let plan = self.rustic.backup_plan(&run.cx, collection.primary)?;
            if !plan.not_backed_up.is_empty() {
                return Err(Error::NotBackedUp { sources: plan.not_backed_up }.into());
            }
            let (files, bytes) = plan.snapshots.iter().fold((0, 0), |(f, b), s| (f + s.files, b + s.bytes));
            return Ok((StepResult::WouldBackUp { repo: collection.name().to_owned(), files, bytes }, Vec::new()));
        }
        let init = self.ensure_primary(run)?;
        let _locks = self.lock(run, true, &[(collection.primary, Use::Add)])?;
        let backup = run.tracked(None, |progress| self.rustic.backup(&run.cx, collection.primary, init, progress));
        if init {
            self.created(run, collection.primary);
        } else {
            run.changed(collection.primary);
        }
        let backup = backup?;
        if !backup.not_backed_up.is_empty() {
            return Err(Error::NotBackedUp { sources: backup.not_backed_up }.into());
        }
        let snapshots = backup
            .snapshots
            .iter()
            .map(|id| self.rustic.snapshot(&run.cx, collection.primary, id))
            .collect::<Result<Vec<_>>>()?;
        self.verify_new_snapshots(run, &snapshots)?;
        let skipped = backup.skipped;
        let skipped = (!skipped.is_empty()).then(|| StepResult::Skipped {
            items: skipped.iter().take(SKIPPED_SHOWN).cloned().collect(),
            count: skipped.len(),
        });
        let parent = backup.unreadable_parent.map(|(path, why)| StepResult::ParentUnreadable {
            repo: collection.name().to_owned(),
            path,
            why,
        });
        let snapshots = snapshots.iter().map(|s| s.id.short().to_owned()).collect();
        Ok((
            StepResult::BackedUp { repo: collection.name().to_owned(), snapshots, new_repository: init },
            parent.into_iter().chain(skipped).collect(),
        ))
    }

    /// Sources that hooks create are recorded where they lead, such as a link's target. A backup tells:
    /// the snapshots that no other source explains are theirs, in profile order, which rustic backs up in.
    /// Learned only when each of them made one, else the order would not tell.
    fn learn_hooked(&self, p: &RusticProfile, snapshots: &[Snapshot]) {
        let known: BTreeSet<String> =
            p.sources.iter().filter(|s| !s.unknown()).map(|s| rustic::source_key(&s.paths())).collect();
        let theirs: Vec<&Snapshot> = snapshots.iter().filter(|sn| !known.contains(&sn.source())).collect();
        let unknown: Vec<&Sources> = p.sources.iter().filter(|s| s.unknown()).collect();
        if unknown.is_empty() || theirs.len() != unknown.len() {
            return;
        }
        for (s, sn) in unknown.into_iter().zip(theirs).filter(|(s, sn)| s.paths() != sn.paths) {
            _ = self.store.set_hooked_paths(&p.name, &s.sources, Some(&sn.paths));
            s.learn(sn.paths.clone());
        }
    }

    /// A collection's snapshots are found by the source paths derived from its profile, through the
    /// profile's `[snapshot-filter]`, which rustic applies to every command. Both must match what rustic
    /// just recorded.
    fn verify_new_snapshots(&self, run: &Run<'_>, snapshots: &[Snapshot]) -> Result<()> {
        self.learn_hooked(run.collection.primary, snapshots);
        let expected: BTreeSet<String> = run.collection.primary.groups().map(|g| rustic::source_key(&g)).collect();
        if let Some(sn) = snapshots.iter().find(|s| !expected.contains(&s.source())) {
            let (snapshot, paths) = (sn.id.short().to_owned(), sn.paths.clone());
            return Err(Error::Unexpected { snapshot, paths, expected }.into());
        }
        // The listing that the copies and the size use next.
        let new: Vec<&Snapshot> = snapshots.iter().collect();
        let listed = self.listed_with(run, run.collection.primary, &new).map_err(|e| Error::UnlistedAfter(e.into()))?;
        if let Some(sn) = snapshots.iter().find(|s| !listed.iter().any(|l| l.same(s))) {
            return Err(Error::Filtered { snapshot: sn.id.short().to_owned() }.into());
        }
        Ok(())
    }

    /// Creates a primary that cannot be listed only for a collection without history: never backed up or seen,
    /// and with no snapshots in any target. Otherwise the repository may be briefly unavailable (an
    /// unmounted disk, whose empty mount point would receive a new repository) or lost. Only an
    /// explicit Re-create replaces it. A new primary takes a target's chunker (`copy --init`), because
    /// copies need matching chunkers. Returns whether the backup must create the repository itself.
    fn ensure_primary(&self, run: &Run<'_>) -> Result<bool> {
        let collection = run.collection;
        let Err(e) = self.list(run, collection.primary) else { return Ok(false) };
        let state = self.store.load();
        let seen = state.succeeded(&StepKey::Snapshot(collection.name().to_owned()))
            || state.latest.contains_key(collection.name());
        let listed: Vec<(&RusticProfile, Vec<Snapshot>)> =
            collection.targets.iter().filter_map(|t| Some((*t, self.list(run, t).ok()?))).collect();
        if seen || listed.iter().any(|(_, snapshots)| !snapshots.is_empty()) {
            let (repo, error) = (collection.primary.name.clone(), format!("{e:#}"));
            return Err(Error::PrimaryUnlisted { repo, error }.into());
        }
        let Some((src, _)) = listed.first() else { return Ok(true) };
        self.copy_into(run, collection.primary, src, &[])?;
        run.push(StepResult::PrimaryCreated { repo: collection.name().to_owned(), from: src.name.clone() });
        Ok(false)
    }

    /// Copies `snapshots` from `src` into `into`, creating `into` with `src`'s chunker.
    pub(super) fn copy_into(
        &self,
        run: &Run<'_>,
        into: &RusticProfile,
        src: &RusticProfile,
        snapshots: &[Snapshot],
    ) -> Result<Vec<Snapshot>> {
        let _locks = self.lock(run, false, &[(into, Use::Add), (src, Use::Read)])?;
        let ids: Vec<&SnapshotId> = snapshots.iter().map(|s| &s.id).collect();
        let said = run.tracked(Some(Doing::CopyFrom { from: src.name.clone() }), |progress| {
            self.rustic.copy(&run.cx, src, &ids, &into.name, true, progress)
        })?;
        self.created(run, into);
        let (repo, from) = (into.name.clone(), src.name.clone());
        let Ok(listed) = self.listed_with(run, into, &snapshots.iter().collect::<Vec<_>>()) else {
            return Err(Error::NotRecreated { repo, from, error: said }.into());
        };
        let copied: Vec<Snapshot> =
            snapshots.iter().filter_map(|s| listed.iter().find(|l| l.same(s)).cloned()).collect();
        if copied.len() < snapshots.len() {
            return Err(Error::NotCopiedInto { from, repo, error: said }.into());
        }
        self.measure(run, into);
        Ok(copied)
    }
}
