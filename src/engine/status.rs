//! The status of each collection and repository, for `collections`, the web UI and notifications.

use std::{collections::BTreeSet, sync::LazyLock};

use jiff::Timestamp;

use super::report::{CollectionOverview, Issue, Last, RepoOverview, RepoStatus, StatusReport, Unconfigured};
use super::{Collection, Engine, Run, each, schedule};
use crate::request::{RunKind, StepKey};
use crate::{
    fmt, live,
    log::{Level, Logged, Message},
    profile::{RusticProfile, Sources},
    result::{RunResults, Status, StepResult},
    settings::{self},
    state::{Checks, Inventory, LastStep, Ledger, RepoInfo},
    sys,
};

impl Engine {
    /// Lists the collection's snapshots and sizes in each of `repos` in parallel, and updates the targets' lag.
    /// Stores the result for the web UI, including failures, and calls `done` for each repository.
    pub(crate) fn list_repos(
        &self,
        collection: &Collection<'_>,
        repos: &[&RusticProfile],
        done: impl Fn(&RusticProfile) + Sync,
    ) {
        // Listed once, at the start, for the primary and for the targets' lag.
        let primary =
            LazyLock::new(|| self.list(&Run::new(collection, None), collection.primary).map_err(|e| format!("{e:#}")));
        std::thread::scope(|scope| {
            // Started at once, beside the other repositories.
            scope.spawn(|| LazyLock::force(&primary));
            each(repos, |repo| {
                let run = Run::new(collection, None);
                let (snapshots, stats) = std::thread::scope(|scope| {
                    let stats = scope.spawn(|| self.rustic.stats(&run.cx, repo));
                    let snapshots = if repo.name == collection.name() {
                        primary.clone().map_err(anyhow::Error::msg)
                    } else {
                        self.list(&run, repo)
                    };
                    (snapshots, crate::thread::joined(stats.join()))
                });
                // A target that cannot be listed is not in sync. The primary has no lag.
                let target = collection.targets.iter().any(|t| t.name == repo.name);
                let lag = (primary.as_ref().ok().filter(|_| target)).map(|primary| {
                    (primary, snapshots.as_ref().ok().map(|s| self.covered(collection, repo, primary, s)))
                });
                let info = snapshots.and_then(|snapshots| {
                    let stats = stats?;
                    Ok(RepoInfo { snapshots, stored: Some(stats.stored()), data: Some(stats.data()) })
                });
                let inventory = Inventory {
                    listed: self.store.now(),
                    result: info.map_err(|e| format!("{e:#}")),
                    size_outdated: false,
                };
                if let Err(e) = self.store.save_inventory(collection.name(), &repo.name, &inventory) {
                    let (repo, error) = (repo.name.clone(), format!("{e:#}"));
                    crate::log::event(Some(collection.title), &Logged::ListingNotKept { repo, error });
                }
                if let Some((primary, latest)) = lag {
                    self.record_lag(&run, repo, primary, latest.as_ref());
                }
                done(repo);
            });
        });
    }

    /// Collections whose primary was never listed (a new state or a new collection): their status is
    /// unknown until it is. A listing, even a failed one, records that it was looked at.
    pub(crate) fn unknown(&self) -> Vec<Collection<'_>> {
        self.collections().into_iter().filter(|j| self.store.inventory(j.name(), j.name()).is_none()).collect()
    }

    /// Lists the repositories of [`Engine::unknown`] collections, as `Refresh all` does.
    pub(crate) fn list_unknown(&self) {
        each(&self.unknown(), |c| self.list_repos(c, &c.repos().collect::<Vec<_>>(), |_| {}));
    }

    /// Lists every repository again, as [`Engine::list_repos`] does, and reports those that cannot be listed.
    pub(crate) fn refresh(&self, collection: &Collection<'_>) -> RunResults {
        let run = Run::new(collection, None);
        self.list_repos(collection, &collection.repos().collect::<Vec<_>>(), |_| {});
        // Not yet created, as before the first backup: its status says so, and it is not unavailable.
        let overview = self.status_of(collection);
        let created = |name: &str| {
            std::iter::once(&overview.primary)
                .chain(&overview.targets)
                .all(|r| r.name != name || r.status != RepoStatus::NotCreated)
        };
        for repo in collection.repos().filter(|r| created(&r.name)) {
            let Some(Err(error)) = self.store.inventory(collection.name(), &repo.name).map(|i| i.result) else {
                continue;
            };
            run.push(if repo.name == collection.name() {
                StepResult::PrimaryUnavailable { repo: repo.name.clone(), error, stopped: false }
            } else {
                StepResult::Unavailable { repo: repo.name.clone(), error }
            });
        }
        run.run_results()
    }

    /// The collection's runs, in this process or in another one.
    pub(crate) fn running(&self, collection: &str) -> Vec<live::Running> {
        let mut running: Vec<live::Running> = self.runs.of(collection).iter().map(|l| l.running()).collect();
        let elsewhere: Vec<_> = (self.store.unfinished(Some(collection)).into_iter())
            .filter(|r| r.status == Status::Running && !running.iter().any(|l| l.id == r.id))
            .filter(|r| self.store.alive(r))
            .collect();
        running.extend(elsewhere.iter().map(live::Running::elsewhere));
        running
    }

    /// Clears an issue of a step's last attempt, if the collection shows it. Returns whether it did.
    #[cfg(feature = "server")]
    pub(crate) fn dismiss(&self, collection: &str, key: StepKey) -> anyhow::Result<bool> {
        let shown = self
            .collection(collection)
            .is_some_and(|j| self.status_of(&j).issues.iter().any(|p| p.step() == Some(&key)));
        if shown {
            self.store.apply(&[crate::state::Event::Dismissed { key }])?;
        }
        Ok(shown)
    }

    /// Logs the unreadable profiles and an untested rustic, and returns the worst level.
    pub(crate) fn report_broken(&self) -> Level {
        for (name, err) in &self.broken {
            crate::log::event(Some(name), &Issue::Unreadable { error: err.clone() });
        }
        let untested = self.rustic.untested();
        if let Some(warning) = &untested {
            crate::log::event(None, warning);
        }
        match (self.broken.is_empty(), untested) {
            (false, _) => Level::Failure,
            (true, Some(_)) => Level::Warning,
            (true, None) => Level::Success,
        }
    }

    /// Every collection's status, for `collections` and the `/status` endpoint.
    pub(crate) fn report(&self) -> StatusReport {
        let (collections, untested, unconfigured) = (self.status(), self.rustic.untested(), self.unconfigured());
        let level = collections
            .iter()
            .map(|j| j.level)
            .chain(untested.as_ref().map(Message::level))
            .chain(unconfigured.as_ref().map(Message::level))
            .max()
            .unwrap_or_default();
        StatusReport { time: self.store.now(), level, untested, unconfigured, collections }
    }

    /// Why nothing is backed up, if no profile makes a collection.
    pub(crate) fn unconfigured(&self) -> Option<Unconfigured> {
        let ran = || self.store.load().ran_before();
        self.every_collection().next().is_none().then(|| if ran() { Unconfigured::Gone } else { Unconfigured::New })
    }

    /// Every collection's status, then the unreadable profiles.
    pub(crate) fn status(&self) -> Vec<CollectionOverview> {
        let (state, now) = (self.store.load(), self.store.now());
        let mut out: Vec<CollectionOverview> =
            self.collections().iter().map(|collection| self.collection_status(collection, &state, now)).collect();
        out.extend(self.broken.iter().map(|(name, err)| CollectionOverview {
            title: name.clone(),
            level: Level::Failure,
            issues: vec![Issue::Unreadable { error: err.clone() }],
            primary: RepoOverview { name: name.clone(), ..RepoOverview::default() },
            ..Default::default()
        }));
        out
    }

    pub(crate) fn status_of(&self, collection: &Collection<'_>) -> CollectionOverview {
        self.collection_status(collection, &self.store.load(), self.store.now())
    }

    /// The levels of `collections`, read from one ledger.
    pub(crate) fn levels(&self, collections: &[Collection<'_>]) -> Vec<Level> {
        let (state, now) = (self.store.load(), self.store.now());
        collections.iter().map(|c| self.collection_status(c, &state, now).level).collect()
    }

    /// `failing`: whether a step's failure is reported, rather than pending.
    fn repo_status(
        &self,
        p: &RusticProfile,
        r: &RusticProfile,
        state: &Ledger,
        now: Timestamp,
        failing: &dyn Fn(&LastStep) -> bool,
    ) -> RepoOverview {
        let last = |key: StepKey| state.last_step_owned(&key);
        let primary = r.name == p.name;
        let (backed_up, last_backup) = schedule::backups(p, state);
        let stale_at = self.stale_at(p, r, state, last_backup);
        if let Some(at) = stale_at.filter(|at| *at > now) {
            crate::clock::changes_at(at);
        }
        let (prune, check) = (last(StepKey::Prune(r.name.clone())), last(StepKey::Check(r.name.clone())));
        let copy = if primary { None } else { last(StepKey::Copy(p.name.clone(), r.name.clone())) };
        let latest_snapshot = state.latest.get(&r.name).copied();
        let shown = |t: &Option<LastStep>, newer| Last::shown(t.as_ref(), newer, failing);
        let (copied, missing) = (shown(&copy, latest_snapshot), !r.exists());
        let lag = state.lag(&p.name, &r.name).cloned().filter(|_| !primary);
        let behind = lag.as_ref().and_then(|l| l.behind_since);
        // It had snapshots once: a backed-up primary, or a target that was copied to or listed with snapshots.
        let created = match primary {
            true => backed_up.is_some(),
            false => latest_snapshot.is_some() || copy.as_ref().is_some_and(|c| c.last_ok.is_some()),
        };
        let status = if missing && created {
            RepoStatus::Missing { primary }
        } else if matches!(copied, Last::Failing(_)) {
            RepoStatus::CopyFailing
        } else if missing {
            RepoStatus::NotCreated
        } else if !primary && lag.is_none() {
            RepoStatus::NotChecked
        } else if stale_at.is_some_and(|at| at <= now) {
            RepoStatus::Stale { behind_since: behind }
        } else if primary && backed_up.is_none() {
            RepoStatus::NotBackedUp
        } else {
            behind.map_or(RepoStatus::InSync, |since| RepoStatus::Behind { since })
        };
        RepoOverview {
            name: r.name.clone(),
            prune_schedule: schedule::of(&[r, p], RunKind::Prune, now, &self.tz),
            check_schedule: schedule::of(&[r, p], RunKind::Check, now, &self.tz),
            repository: r.repository.as_deref().map(fmt::redact),
            latest_snapshot,
            next_subset: super::maintain::subsets(&[r, p])
                .and_then(|_| state.checks.get(&r.name).map(Checks::upcoming)),
            check_subsets: settings::CHECK_SUBSETS.get(&[r, p]),
            pruned: shown(&prune, None),
            checked: shown(&check, None),
            copied,
            prune,
            check,
            forget: last(StepKey::Forget(p.name.clone(), r.name.clone())),
            copy,
            lag,
            missing,
            stale_at,
            status,
        }
    }

    pub(super) fn collection_status(
        &self,
        collection: &Collection<'_>,
        state: &Ledger,
        now: Timestamp,
    ) -> CollectionOverview {
        let p = collection.primary;
        let snapshot_key = StepKey::Snapshot(p.name.clone());
        let snapshot = state.last_step_owned(&snapshot_key);
        let (backed_up, last_backup) = schedule::backups(p, state);
        let running = self.running(&p.name);
        let failing = |t: &LastStep| {
            // A failure awaiting its retry shows as failing once the retry is late.
            if let Some(until) = t.pending_until().filter(|until| *until > now) {
                crate::clock::changes_at(until);
            }
            t.failing(now, !running.is_empty())
        };
        let repo = |r: &RusticProfile| self.repo_status(p, r, state, now, &failing);
        let primary = repo(p);
        let targets: Vec<RepoOverview> = collection.targets.iter().map(|t| repo(t)).collect();
        let mut issues: Vec<Issue> =
            collection.missing_targets.iter().map(|t| Issue::CopyTargetUnusable(self.unusable(t))).collect();
        issues.extend(
            collection.repos().filter(|r| r.prunes_on_forget).map(|r| Issue::PrunesOnForget { repo: r.name.clone() }),
        );
        issues
            .extend(collection.repos().filter(|r| !r.has_password).map(|r| Issue::NoPassword { repo: r.name.clone() }));
        issues.extend(
            (collection.targets.iter().filter(|t| t.repo_key() == p.repo_key()))
                .map(|t| Issue::SameRepository { target: t.name.clone(), primary: p.name.clone() }),
        );
        // An unknown setting is likely a typo, which leaves a step unscheduled.
        issues.extend(collection.repos().flat_map(|r| {
            settings::ignored(r, &[p])
                .into_iter()
                .map(|setting| Issue::SettingIgnored { repo: r.name.clone(), setting })
        }));
        issues.extend(Issue::failed(snapshot.as_ref().filter(|t| failing(t)), &snapshot_key));
        if let Some(t) = snapshot.as_ref().filter(|t| !t.warnings.is_empty()) {
            issues.push(Issue::Warned { step: snapshot_key, warnings: t.warnings.clone(), by: t.by.clone() });
        }
        let sources = p.sources.iter().flat_map(Sources::paths);
        issues
            .extend(sources.filter(|s| sys::needs_disk_access(s)).map(|source| Issue::NoDiskAccess { source }).take(1));
        // Unknown, not "never", until the repository is first listed.
        let listed = self.store.inventory(&p.name, &p.name).is_some();
        if listed && backed_up.is_none() && snapshot.as_ref().is_none_or(|t| !failing(t)) {
            issues.push(Issue::NeverBackedUp);
        }
        for r in std::iter::once(&primary).chain(&targets) {
            issues.extend(r.issues(&p.name, last_backup, &failing));
        }
        let backup_schedule = schedule::of(&[p], RunKind::Backup, now, &self.tz);
        // Each error once, also when every repository inherits it from the backup profile's rustic profile.
        let invalid: BTreeSet<String> = std::iter::once(&backup_schedule)
            .chain(std::iter::once(&primary).chain(&targets).flat_map(|r| [&r.prune_schedule, &r.check_schedule]))
            .filter_map(|s| s.as_ref()?.error.clone())
            .collect();
        issues.extend(invalid.into_iter().map(|error| Issue::InvalidSchedule { error }));
        if settings::NOTIFY.get(&[p]).is_some()
            && let Ok(notified) = self.store.notified(&p.name)
        {
            issues.extend(notified.error.filter(|_| notified.failures > 0).map(|error| Issue::Undelivered { error }));
        }
        // A missing repository is the one issue its steps' failures and warnings repeat, and those of the copies
        // that read a missing primary.
        let missing: Vec<String> = issues
            .iter()
            .filter_map(|i| if let Issue::Missing { repo, .. } = i { Some(repo.clone()) } else { None })
            .collect();
        let repeats =
            |s: &StepKey| missing.iter().any(|m| m == s.repo() || matches!(s, StepKey::Copy(..)) && *m == p.name);
        issues.retain(|i| i.step().is_none_or(|s| !repeats(s)));
        issues.sort_by_key(|p| std::cmp::Reverse(p.level()));
        CollectionOverview {
            title: collection.title.to_owned(),
            level: issues.iter().filter(|p| p.counts()).map(Issue::level).max().unwrap_or_default(),
            issues,
            last_backup,
            backup_schedule,
            snapshot,
            primary,
            targets,
            running,
        }
    }
}
