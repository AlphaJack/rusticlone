//! The collection's snapshots in each repository: listing, measuring and picking them.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use anyhow::Result;
use jiff::Timestamp;

use super::{Engine, Run};
use crate::result::Doing;
use crate::{
    log::Logged,
    profile::{self, RusticProfile, Sources},
    rustic::{self, Snapshot, SnapshotGroup},
};

impl Engine {
    /// Stores its size for the web UI, after a run changed it.
    pub(super) fn measure(&self, run: &Run<'_>, repo: &RusticProfile) {
        match self.rustic.stats(&run.cx, repo) {
            Ok(info) => self.store.measured(run.collection.name(), &repo.name, info.stored(), info.data()),
            Err(e) => {
                let (repo, error) = (repo.name.clone(), format!("{e:#}"));
                crate::log::event(Some(run.collection.title), &Logged::NotMeasured { repo, error });
            }
        }
    }

    /// The collection's snapshots in a repository, once they include `wanted`, which was just written. A
    /// repository whose listings may lag is listed again after a pause, a few times.
    pub(super) fn listed_with(&self, run: &Run<'_>, p: &RusticProfile, wanted: &[&Snapshot]) -> Result<Vec<Snapshot>> {
        let holds = |listed: &[Snapshot]| wanted.iter().all(|w| listed.iter().any(|l| l.same(w)));
        let mut listed = self.list(run, p)?;
        for secs in SETTLE.iter().take_while(|_| p.may_lag()) {
            if holds(&listed) || run.cx.stopping() {
                break;
            }
            run.activity(Some(Doing::WaitingForListing { repo: p.name.clone() }), false);
            self.store.wait(Duration::from_secs(*secs), &|| run.cx.stopping());
            run.idle();
            run.changed(p);
            listed = self.list(run, p)?;
        }
        Ok(listed)
    }

    /// [`Self::learn_from_repository`] from each repository in turn, as a restore may find the primary lost.
    pub(super) fn learn_from_repositories(&self, run: &Run<'_>) {
        let unlearned = || run.collection.primary.sources.iter().any(|s| s.unknown() && !s.learned());
        for repo in run.collection.repos().take_while(|_| unlearned()) {
            _ = self.learn_from_repository(run, repo);
        }
    }

    /// Sources that hooks create, when nothing learned their paths yet, as on a new computer: the newest
    /// snapshots of their hosts that no other profile's source explains, in the order rustic made them,
    /// which is the profile's. Learned only when there are enough of them.
    fn learn_from_repository(&self, run: &Run<'_>, p: &RusticProfile) -> Result<()> {
        let primary = run.collection.primary;
        let unknown: Vec<&Sources> = primary.sources.iter().filter(|s| s.unknown() && !s.learned()).collect();
        if unknown.is_empty() {
            return Ok(());
        }
        // Other profiles may share the repository, also those that this run did not read: read them now, as
        // rustic does, which costs a call each, but learning is rare.
        let unread: Vec<RusticProfile> = (profile::profile_files()?.into_iter())
            .filter(|(name, _)| !self.rustic_profiles.contains_key(name))
            .filter_map(|(name, dir)| profile::load(&name, &dir, &self.rustic.config(&name, &dir).ok()?).ok())
            .collect();
        let known: BTreeSet<String> = (self.rustic_profiles.values().chain(&unread).flat_map(|q| &q.sources))
            .filter(|s| !s.unknown())
            .map(|s| rustic::source_key(&s.paths()))
            .collect();
        let groups = self.rustic.snapshots(&run.cx, p, std::iter::empty::<Vec<String>>(), &primary.group_by)?;
        let theirs = (groups.iter().flat_map(|g| &g.snapshots))
            .filter(|sn| !known.contains(&sn.source()) && unknown.iter().any(|s| s.host == sn.hostname));
        let mut picks: Vec<&Snapshot> = newest_by_source(theirs).into_values().collect();
        picks.sort_by_key(|sn| Reverse(sn.time));
        picks.truncate(unknown.len());
        if picks.len() < unknown.len() {
            return Ok(());
        }
        picks.sort_by_key(|sn| sn.time);
        for (s, sn) in unknown.into_iter().zip(picks) {
            _ = self.store.set_hooked_paths(&primary.name, &s.sources, Some(&sn.paths));
            s.learn(sn.paths.clone());
        }
        Ok(())
    }

    /// The collection's snapshots in a repository, also stored for the web UI so that it needs no listing of
    /// its own.
    pub(super) fn list(&self, run: &Run<'_>, p: &RusticProfile) -> Result<Vec<Snapshot>> {
        if let Some(snapshots) = run.shared.listed.lock().get(&p.name) {
            return Ok(snapshots.clone());
        }
        // Its listing fails the same way next, if it cannot list.
        _ = self.learn_from_repository(run, p);
        let doing = Doing::Listing { repo: p.name.clone() };
        // A step shows it as what it does now; outside steps, it has its own line.
        let in_step = run.live.is_some() && run.cx.step().is_some();
        if in_step {
            run.activity(Some(doing), false);
        } else {
            crate::log::doing(run.collection.title, &doing);
        }
        let collection = run.collection.primary;
        // Also as the profile writes them: another computer recorded its paths without this one's links.
        let mut groups: Vec<Vec<String>> = collection.groups().collect();
        for written in collection.sources.iter().map(Sources::written_paths) {
            if !groups.contains(&written) {
                groups.push(written);
            }
        }
        let listed = self.rustic.snapshots(&run.cx, p, groups, &collection.group_by);
        if in_step {
            run.idle();
        } else {
            crate::log::clear();
        }
        let snapshots = own(collection, &listed?);
        self.store.relisted(run.collection.name(), &p.name, &snapshots, false);
        self.note_latest(&p.name, newest(&snapshots));
        run.shared.listed.lock().insert(p.name.clone(), snapshots.clone());
        Ok(snapshots)
    }
}

/// Seconds to wait each time for a lagging repository to list what was written.
const SETTLE: [u64; 3] = [5, 15, 40];

/// The collection's snapshots among the groups rustic made with the collection's `group-by`: one group per source.
/// The group of this host, else the one with the newest snapshot (the computer was reinstalled with
/// another name). A `group-by` without `host` already merges every host.
pub(super) fn own(collection: &RusticProfile, groups: &[SnapshotGroup]) -> Vec<Snapshot> {
    let mut own = Vec::new();
    for s in &collection.sources {
        // As this computer derives its paths, or as the profile writes them for another computer's snapshots.
        let sources = [rustic::source_key(&s.paths()), rustic::source_key(&s.written_paths())];
        let of_source = |g: &SnapshotGroup| {
            g.snapshots.iter().filter(|sn| sources.contains(&sn.source())).cloned().collect::<Vec<_>>()
        };
        let here = |g: &SnapshotGroup| g.group_key.hostname.as_ref().is_none_or(|h| *h == s.host);
        let chosen = groups
            .iter()
            .map(|g| (here(g), of_source(g)))
            .filter(|(_, s)| !s.is_empty())
            .max_by_key(|(here, s)| (*here, newest(s)));
        own.extend(chosen.into_iter().flat_map(|(_, s)| s));
    }
    own
}

/// Below this size, a source is too small to detect that it shrank.
const SHRINK_FLOOR: u64 = 1 << 20;

/// Each source whose newest snapshot holds less than 1% of an older one, as when its disk did not
/// mount or the computer was reinstalled: the newest snapshot, and those older ones. Forgetting them
/// would leave only the empty snapshot, so they are kept until a new snapshot holds at least 1% of
/// their size.
pub(super) fn shrunk(snapshots: &[Snapshot]) -> Vec<(&Snapshot, Vec<&Snapshot>)> {
    let size = |s: &Snapshot| s.summary.as_ref().map(|s| s.total_bytes_processed);
    let sources: BTreeSet<String> = snapshots.iter().map(Snapshot::source).collect();
    sources
        .into_iter()
        .filter_map(|source| {
            let of: Vec<&Snapshot> = snapshots.iter().filter(|s| s.source() == source).collect();
            let newest = *of.iter().max_by_key(|s| s.time)?;
            let small = size(newest)?;
            let held: Vec<&Snapshot> =
                of.into_iter().filter(|s| size(s).is_some_and(|b| b >= SHRINK_FLOOR && b / 100 > small)).collect();
            (!held.is_empty()).then_some((newest, held))
        })
        .collect()
}

/// The newest snapshot of each source.
pub(super) fn newest_by_source<'a>(
    snapshots: impl IntoIterator<Item = &'a Snapshot>,
) -> BTreeMap<String, &'a Snapshot> {
    let mut newest: BTreeMap<String, &Snapshot> = BTreeMap::new();
    for sn in snapshots {
        let entry = newest.entry(sn.source()).or_insert(sn);
        if sn.time > entry.time {
            *entry = sn;
        }
    }
    newest
}

pub(super) fn newest(snapshots: &[Snapshot]) -> Option<Timestamp> {
    snapshots.iter().map(|s| s.time).max()
}

/// The newest snapshot of each group, not after `at`.
pub(super) fn pick(snapshots: &[Snapshot], groups: &[Vec<String>], at: Option<Timestamp>) -> Vec<Snapshot> {
    groups
        .iter()
        .filter_map(|g| {
            let key = rustic::source_key(g);
            snapshots
                .iter()
                .filter(|s| s.source() == key && at.is_none_or(|at| s.time <= at))
                .max_by_key(|s| s.time)
                .cloned()
        })
        .collect()
}

/// Calls `f` for each item in parallel, so that rustic calls, which mostly wait for a remote, do not
/// wait for each other.
pub(super) fn each<I: Sync, T: Send>(items: &[I], f: impl Fn(&I) -> T + Sync) -> Vec<T> {
    std::thread::scope(|scope| {
        let threads: Vec<_> = items.iter().map(|item| scope.spawn(|| f(item))).collect();
        threads.into_iter().map(|t| t.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))).collect()
    })
}
