//! Copies to the targets, and forgets only what every target has.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::Ordering,
};

use anyhow::Result;
use jiff::Timestamp;

use super::{Collection, Engine, Error, Run, Use, newest, newest_by_source, shrunk};
use crate::{
    profile::RusticProfile,
    request::StepKey,
    result::StepResult,
    rustic::{self, Snapshot, SnapshotId},
    state::{Behind, Event},
};

impl Run<'_> {
    /// Whether the run already said, in a dry run, that it would re-create `t`.
    fn recreates(&self, t: &RusticProfile) -> bool {
        let label = &t.name;
        self.reported(|r| matches!(r, StepResult::WouldRecreate { repo } if repo == label))
    }
}

impl Engine {
    /// Keeps what the target then holds in the run, for the forgets.
    pub(super) fn copy_step(&self, run: &Run<'_>, t: &RusticProfile) {
        run.shared.copying.store(true, Ordering::Relaxed);
        // A dry run's first backup would create the primary, with a snapshot per source for every target.
        if self.store.is_dry_run() && run.reported(|r| matches!(r, StepResult::WouldCreatePrimary { .. })) {
            let (snapshots, create) = (run.collection.primary.sources.len() as u64, self.list(run, t).is_err());
            return run.push(StepResult::WouldCopy { target: t.name.clone(), snapshots, create });
        }
        let Some(primary) = self.copy_source(run) else { return };
        if let Some(latest) = self.copy(run, t, &primary) {
            run.shared.copied.lock().insert(t.name.clone(), latest);
        }
    }

    /// The primary's snapshots to copy. If the primary cannot be listed, the copy fails, so that it is
    /// retried with backoff rather than every minute. The error is reported once for all targets.
    fn copy_source(&self, run: &Run<'_>) -> Option<Vec<Snapshot>> {
        let started = self.store.now();
        let e = match self.list(run, run.collection.primary) {
            Ok(primary) => return Some(primary),
            Err(e) => StepResult::PrimaryUnavailable {
                repo: run.collection.name().to_owned(),
                error: format!("{e:#}"),
                stopped: run.cx.stopping(),
            },
        };
        run.push_once(e.clone());
        self.record(run, started, &e);
        None
    }

    /// Applies the primary's `[forget]` policy, after every copy (see the plan). Snapshots newer than
    /// what every target has are kept, whatever the policy says.
    pub(super) fn forget_primary(&self, run: &Run<'_>) {
        let collection = run.collection;
        // The copies already reported why it cannot be listed.
        let Ok(primary) = self.list(run, collection.primary) else { return };
        let copied = run.shared.copied.lock().clone();
        let latest = collection.targets.iter().filter_map(|t| copied.get(&t.name));
        // Per group, the newest snapshot that every target has.
        let horizon = latest.cloned().reduce(|h, latest| {
            h.into_iter().filter_map(|(g, ts)| latest.get(&g).map(|t2| (g, ts.min(*t2)))).collect()
        });
        let held: Vec<&Snapshot> = horizon.map(|h| missing(&primary, &h).collect()).unwrap_or_default();
        self.forget(run, collection.primary, &held);
    }

    /// The target's newest snapshot per group, if the copy was confirmed.
    fn copy(&self, run: &Run<'_>, t: &RusticProfile, primary: &[Snapshot]) -> Option<BTreeMap<String, Timestamp>> {
        let collection = run.collection;
        let started = self.store.now();
        let result = self.copy_inner(run, t, primary);
        let latest = result.as_ref().ok().map(|(_, latest)| latest.clone());
        self.record_lag(run, t, primary, latest.as_ref());
        if latest.is_some() {
            self.emit(
                collection.name(),
                &[Event::Copied { collection: collection.name().to_owned(), target: t.name.clone() }],
            );
        }
        self.step(run, started, result.map(|(note, _)| note));
        latest
    }

    /// Copies the primary's snapshots that are newer than the target's newest of their group. Older ones
    /// are never sent again, so a target may keep a shorter history.
    fn copy_inner(
        &self,
        run: &Run<'_>,
        t: &RusticProfile,
        primary: &[Snapshot],
    ) -> Result<(StepResult, BTreeMap<String, Timestamp>)> {
        let collection = run.collection;
        let _locks = self.lock(run, false, &[(collection.primary, Use::Read), (t, Use::Add)])?;
        let target = match self.list(run, t) {
            Ok(s) => s,
            Err(e) if run.cx.stopping() => return Err(e),
            // Briefly unavailable, or deleted: only Re-create brings it back.
            // A dry run creates no repository, here or in an earlier re-create: it would get every snapshot.
            Err(_) if self.store.is_dry_run() && (!self.seen(collection, t) || run.recreates(t)) => {
                let snapshots = missing(primary, &BTreeMap::new()).count() as u64;
                return Ok((
                    StepResult::WouldCopy { target: t.name.clone(), snapshots, create: true },
                    BTreeMap::new(),
                ));
            }
            Err(e) if self.seen(collection, t) => {
                return Err(Error::Unavailable { repo: t.name.clone(), error: format!("{e:#}") }.into());
            }
            // Never seen: create it. If it still cannot be listed, report why.
            Err(_) => {
                self.create_target(run, t)?;
                self.list(run, t).map_err(|e| Error::Unreachable(e.into()))?
            }
        };
        let latest = self.covered(collection, t, primary, &target);
        let missing: Vec<&Snapshot> = missing(primary, &latest).collect();
        if missing.is_empty() {
            return Ok((StepResult::InSync { target: t.name.clone() }, latest));
        }
        if self.store.is_dry_run() {
            let would =
                StepResult::WouldCopy { target: t.name.clone(), snapshots: missing.len() as u64, create: false };
            return Ok((would, latest));
        }
        let ids: Vec<&SnapshotId> = missing.iter().map(|s| &s.id).collect();
        let said =
            run.tracked(None, |progress| self.rustic.copy(&run.cx, collection.primary, &ids, &t.name, false, progress));
        run.changed(t);
        let said = said?;
        // Rustic exits 0 even when a target fails.
        let Ok(after) = self.listed_with(run, t, &missing) else { return Err(rustic::Error::Failed(said).into()) };
        let failed = missing.iter().filter(|s| !after.iter().any(|a| a.same(s))).count();
        if failed > 0 {
            return Err(Error::NotCopied { failed, of: missing.len(), error: said }.into());
        }
        self.measure(run, t);
        let copied = StepResult::Copied { target: t.name.clone(), snapshots: missing.len() as u64 };
        Ok((copied, self.covered(collection, t, primary, &after)))
    }

    /// Whether the target was copied to or listed before. If so, a target that cannot be listed
    /// is not new.
    fn seen(&self, collection: &Collection<'_>, t: &RusticProfile) -> bool {
        let state = self.store.load();
        state.succeeded(&StepKey::Copy(collection.name().to_owned(), t.name.clone()))
            || state.lag(collection.name(), &t.name).is_some_and(|l| l.listed)
            || state.latest.contains_key(&t.name)
    }

    /// Creates the target empty, with the primary's chunker (`copy --init`). The caller holds the copy's
    /// locks.
    pub(super) fn create_target(&self, run: &Run<'_>, t: &RusticProfile) -> Result<()> {
        self.rustic.copy(&run.cx, run.collection.primary, &[], &t.name, true, &mut |_| {})?;
        self.created(run, t);
        Ok(())
    }

    /// A repository was created: its id is unknown, and it must be listed again.
    pub(super) fn created(&self, run: &Run<'_>, repo: &RusticProfile) {
        _ = self.store.set_repo_id(repo.repo_key(), None);
        run.changed(repo);
    }

    /// Per group, up to when the target has the primary's snapshots: its newest one, extended past
    /// snapshots its own forget removed (`delete-unchanged`). Otherwise those would be copied and removed
    /// again on every run.
    pub(super) fn covered(
        &self,
        collection: &Collection<'_>,
        t: &RusticProfile,
        primary: &[Snapshot],
        target: &[Snapshot],
    ) -> BTreeMap<String, Timestamp> {
        let dropped: BTreeSet<SnapshotId> = (self.store.load().dropped)
            .remove(&(collection.name().to_owned(), t.name.clone()))
            .into_iter()
            .flatten()
            .map(|(id, _)| id)
            .collect();
        newest_by_source(target)
            .into_iter()
            .map(|(group, last)| {
                let mut later: Vec<&Snapshot> =
                    primary.iter().filter(|s| s.source() == group && s.time > last.time).collect();
                later.sort_by_key(|s| s.time);
                let dropped = later.into_iter().take_while(|s| dropped.contains(s.identity()));
                (group, dropped.map(|s| s.time).last().unwrap_or(last.time))
            })
            .collect()
    }

    /// `latest`: the target's newest snapshot per group, if known.
    pub(super) fn record_lag(
        &self,
        run: &Run<'_>,
        t: &RusticProfile,
        primary: &[Snapshot],
        latest: Option<&BTreeMap<String, Timestamp>>,
    ) {
        let behind = match latest {
            Some(latest) => Behind::Listed(missing(primary, latest).map(|s| s.time).min()),
            // Unknown: behind since the first unconfirmed snapshot.
            None => Behind::Unlisted(newest(primary)),
        };
        let (collection, target, at) = (run.collection.name().to_owned(), t.name.clone(), self.store.now());
        self.emit(run.collection.name(), &[Event::Lag { collection, target, at, behind }]);
        self.note_latest(&t.name, latest.and_then(|l| l.values().max().copied()));
    }

    /// Applies the repository's `[forget]` policy to the collection's sources in one rustic call. `held`
    /// (snapshots not yet on every target) are kept whatever the policy says.
    pub(super) fn forget(&self, run: &Run<'_>, repo: &RusticProfile, held: &[&Snapshot]) {
        let collection = run.collection;
        let what = &repo.name;
        let started = self.store.now();
        let result = (|| -> Result<StepResult> {
            let _lock = self.lock(run, false, &[(repo, Use::Forget)])?;
            let listed = match self.list(run, repo) {
                // A repository that a dry run would create has nothing to forget.
                Err(_) if self.store.is_dry_run() => {
                    return Ok(StepResult::WouldForget { repo: what.to_owned(), snapshots: 0 });
                }
                listed => listed?,
            };
            let shrunk: Vec<&SnapshotId> =
                shrunk(&listed).into_iter().flat_map(|(_, held)| held).map(|s| &s.id).collect();
            let held: Vec<&SnapshotId> = held.iter().map(|s| &s.id).chain(shrunk.iter().copied()).collect();
            run.activity(None, true);
            let groups = self.rustic.forget(&run.cx, repo, collection.primary.groups(), &held);
            run.idle();
            let groups = groups?;
            let items: Vec<_> = groups.iter().flat_map(|g| &g.items).collect();
            let kept: Vec<Snapshot> = items.iter().filter(|i| i.keep).map(|i| i.snapshot.clone()).collect();
            // Forget removes only snapshot files: the size changes with the next prune.
            self.store.relisted(collection.name(), &repo.name, &kept, true);
            run.shared.listed.lock().insert(repo.name.clone(), kept);
            if repo.name != collection.name() {
                let removed: Vec<_> = items
                    .iter()
                    .filter(|i| !i.keep)
                    .map(|i| (i.snapshot.identity().clone(), i.snapshot.time))
                    .collect();
                let oldest_kept = items.iter().filter(|i| i.keep).map(|i| i.snapshot.time).min();
                let (collection, target) = (collection.name().to_owned(), repo.name.clone());
                self.emit(
                    &collection,
                    &[Event::Dropped { collection: collection.clone(), target, removed, oldest_kept }],
                );
            }
            let removed = items.iter().filter(|i| !i.keep).count();
            // Kept only because they were held.
            let only_held: Vec<_> = items.iter().filter(|i| i.keep && i.reasons == ["id"]).collect();
            let kept_shrunk = only_held.iter().filter(|i| shrunk.contains(&&i.snapshot.id)).count();
            let (repo, held) = (what.to_owned(), only_held.len() - kept_shrunk);
            Ok(match self.store.is_dry_run() {
                true => StepResult::WouldForget { repo, snapshots: removed as u64 },
                false => StepResult::Forgot { repo, snapshots: removed as u64, held, shrunk: kept_shrunk },
            })
        })();
        self.step(run, started, result);
    }
}

/// The primary's snapshots newer than the newest one of their group in `held`.
fn missing<'a>(primary: &'a [Snapshot], held: &BTreeMap<String, Timestamp>) -> impl Iterator<Item = &'a Snapshot> {
    primary.iter().filter(|s| held.get(&s.source()).is_none_or(|t| s.time > *t))
}
