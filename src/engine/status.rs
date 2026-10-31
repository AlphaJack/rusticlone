//! The status of each collection and repository, for `collections`, the web UI and notifications.

use std::{collections::BTreeSet, sync::LazyLock};

use jiff::Timestamp;
use serde::Serialize;

use super::{Collection, Engine, Run, Schedule, each, schedule};
use crate::request::{RunKind, StepKey};
use crate::result::Unusable;
use crate::{
    fmt,
    i18n::t,
    live,
    log::{Level, Logged, Message},
    profile::{RusticProfile, Sources},
    result::{RunResults, Status, StepResult},
    rustic::Untested,
    settings::{self, Ignored},
    shape,
    state::{Checks, Inventory, Lag, LastStep, Ledger, RepoInfo, ResultRecord, RunStep},
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
        let lagged = repos.iter().any(|r| r.name != collection.name());
        std::thread::scope(|scope| {
            if lagged {
                scope.spawn(|| LazyLock::force(&primary));
            }
            each(repos, |repo| {
                let run = Run::new(collection, None);
                let (snapshots, stats) = std::thread::scope(|scope| {
                    let stats = scope.spawn(|| self.rustic.stats(&run.cx, repo));
                    let snapshots = if repo.name == collection.name() {
                        primary.clone().map_err(anyhow::Error::msg)
                    } else {
                        self.list(&run, repo)
                    };
                    (snapshots, stats.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic)))
                });
                // A target that cannot be listed is not in sync.
                let lag = match &*primary {
                    Ok(primary) if repo.name != collection.name() => {
                        Some((primary, snapshots.as_ref().ok().map(|s| self.covered(collection, repo, primary, s))))
                    }
                    _ => None,
                };
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
        let last = |key: StepKey| state.last_step(&key).cloned();
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
            next_subset: super::maintain::subsets(&[r, p]).and(state.checks.get(&r.name).map(Checks::upcoming)),
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
        let last = |key: StepKey| state.last_step(&key).cloned();
        let snapshot_key = StepKey::Snapshot(p.name.clone());
        let snapshot = last(snapshot_key.clone());
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
        if settings::NOTIFY.get(&[p]).is_some() {
            let notified = self.store.notified(&p.name);
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

/// No profile makes a collection, so nothing is backed up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, crate::i18n::Fluent)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum Unconfigured {
    /// Nothing was set up yet.
    New,
    /// Collections ran before, and their profiles are gone.
    Gone,
}

impl Message for Unconfigured {
    fn level(&self) -> Level {
        match self {
            Self::New => Level::Warning,
            Self::Gone => Level::Failure,
        }
    }
}

/// Serialized with [`shape::serialize`], so that programs know its version.
#[derive(Serialize)]
pub(crate) struct StatusReport {
    pub time: Timestamp,
    /// The worst level of all collections, `untested` and `unconfigured`.
    pub level: Level,
    #[serde(serialize_with = "crate::log::serialize_message")]
    pub untested: Option<Untested>,
    #[serde(serialize_with = "crate::log::serialize_message", skip_serializing_if = "Option::is_none")]
    pub unconfigured: Option<Unconfigured>,
    pub collections: Vec<CollectionOverview>,
}

impl shape::Versioned for StatusReport {
    const MIGRATIONS: &'static [shape::Migration] = &[shape::Migration { up: activities, read_by_previous: false }];
}

/// A running run's activity as the first of its steps' activities, which run at the same time.
fn activities(report: &mut serde_json::Value) {
    let collections = report.get_mut("collections").and_then(serde_json::Value::as_array_mut);
    for run in collections.into_iter().flatten().filter_map(|c| c.get_mut("running")?.as_array_mut()).flatten() {
        if let Some(run) = run.as_object_mut() {
            let activity = run.remove("activity").filter(|a| !a.is_null());
            run.insert("activities".into(), serde_json::Value::Array(activity.into_iter().collect()));
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, crate::i18n::Fluent)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub(crate) enum RepoStatus {
    /// It had snapshots once, and is gone. A target's backups are still in the primary, as when it cannot
    /// be reached.
    Missing {
        primary: bool,
    },
    CopyFailing,
    NotCreated,
    /// A target whose lag was never measured.
    NotChecked,
    /// Out of sync for longer than the backup schedule allows.
    Stale {
        behind_since: Option<Timestamp>,
    },
    NotBackedUp,
    /// A target that lacks snapshots, within what the schedule allows.
    Behind {
        since: Timestamp,
    },
    InSync,
    /// Not listed: the profile cannot be read.
    #[default]
    Unknown,
}

impl RepoStatus {
    #[cfg(feature = "server")]
    pub(crate) fn level(self) -> Level {
        match self {
            Self::Missing { primary: true } => Level::Failure,
            // The backup is safe in the primary. The copy's step and issue report the failure.
            Self::Missing { primary: false } | Self::CopyFailing | Self::Stale { .. } => Level::Warning,
            Self::NotCreated
            | Self::NotChecked
            | Self::NotBackedUp
            | Self::Behind { .. }
            | Self::InSync
            | Self::Unknown => Level::Success,
        }
    }

    /// Without an outcome yet: muted.
    #[cfg(feature = "server")]
    pub(crate) fn tone(self) -> crate::log::Tone {
        match self {
            Self::NotCreated | Self::NotChecked | Self::NotBackedUp | Self::Unknown => crate::log::Tone::Muted,
            Self::Missing { .. } | Self::CopyFailing | Self::Stale { .. } | Self::Behind { .. } | Self::InSync => {
                self.level().into()
            }
        }
    }
}

#[derive(Default, Serialize)]
pub(crate) struct RepoOverview {
    pub name: String,
    pub prune_schedule: Option<Schedule>,
    pub check_schedule: Option<Schedule>,
    /// Its location, without credentials.
    pub repository: Option<String>,
    pub latest_snapshot: Option<Timestamp>,
    /// The data subset the next check reads.
    pub next_subset: Option<u32>,
    pub check_subsets: Option<String>,
    pub prune: Option<LastStep>,
    pub check: Option<LastStep>,
    pub forget: Option<LastStep>,
    /// Copy targets only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copy: Option<LastStep>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lag: Option<Lag>,
    /// The repository does not exist: deleted, or not created yet.
    pub missing: bool,
    /// When it becomes stale if nothing changes: out of sync for longer than the backup schedule allows.
    pub stale_at: Option<Timestamp>,
    pub status: RepoStatus,
    /// Its last prune, check and copy (to a target, or by other means) as shown: a pending failure shows the
    /// last success.
    #[serde(skip)]
    pub pruned: Last,
    #[serde(skip)]
    pub checked: Last,
    #[serde(skip)]
    pub copied: Last,
}

impl RepoOverview {
    /// `failing`: whether a step's failure is reported, rather than pending.
    fn issues(
        &self,
        collection: &str,
        last_backup: Option<Timestamp>,
        failing: &dyn Fn(&LastStep) -> bool,
    ) -> Vec<Issue> {
        let primary = self.name == collection;
        let name = &self.name;
        let (collection, repo) = (collection.to_owned(), self.name.clone());
        let mut out = Vec::new();
        let last_steps = [
            (&self.copy, StepKey::Copy(collection.clone(), repo.clone())),
            (&self.forget, StepKey::Forget(collection, repo.clone())),
            (&self.prune, StepKey::Prune(repo.clone())),
            (&self.check, StepKey::Check(repo)),
        ];
        // A failing copy is reported below, with the lag.
        for (last, key) in last_steps.iter().skip(1) {
            out.extend(Issue::failed(last.as_ref().filter(|t| failing(t)), key));
        }
        let copy_failed =
            self.copy.as_ref().filter(|t| failing(t)).and_then(|t| Some((t.last_error.clone()?, t.by.clone())));
        let copy_failed = copy_failed.map(|(error, by)| Issue::CopyFailed { step: last_steps[0].1.clone(), error, by });
        let stale = |since| Issue::Stale { repo: name.to_owned(), since };
        match (copy_failed, self.lag.as_ref().and_then(|l| l.behind_since), last_backup) {
            _ if let RepoStatus::Missing { primary } = self.status => {
                out.push(Issue::Missing { repo: name.to_owned(), primary });
            }
            (Some(e), ..) => out.push(e),
            // A lag within the schedule is normal.
            _ if !matches!(self.status, RepoStatus::Stale { .. }) => {}
            (None, Some(since), _) => out.push(stale(Since::Behind(since))),
            // A target in sync is as stale as the primary, which reports it once.
            (None, None, Some(last)) if primary => out.push(stale(Since::LastBackup(last))),
            // Stale with the primary, which reports it.
            _ => {}
        }
        out
    }
}

/// Something in a collection that needs attention. `step`: the step whose last attempt it concerns, which
/// dismissing clears.
#[derive(Debug, Clone, Serialize, strum::IntoStaticStr, crate::i18n::Fluent)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub(crate) enum Issue {
    /// A profile that rustic cannot read.
    #[fluent(transparent = error)]
    Unreadable {
        error: String,
    },
    #[fluent(transparent)]
    CopyTargetUnusable(Unusable),
    /// Not unsafe, because forget keeps what some copy still lacks. But slow, and not scheduled.
    #[fluent(key = settings::PRUNE.key)]
    PrunesOnForget {
        repo: String,
    },
    SettingIgnored {
        repo: String,
        setting: Ignored,
    },
    /// Rustic would ask for the password, which fails without a terminal.
    NoPassword {
        repo: String,
    },
    /// A copy target that is the primary's repository, as through the same `use-profiles`: its copies keep no
    /// second copy, and its `[forget]` policy applies to the primary.
    SameRepository {
        target: String,
        primary: String,
    },
    #[fluent(transparent = error)]
    Failed {
        step: StepKey,
        error: ResultRecord,
        /// The run and step that failed.
        by: Option<RunStep>,
    },
    /// Only a warning: the backup is safe in the primary, and the lag shows how far behind the target is.
    #[fluent(transparent = error)]
    CopyFailed {
        step: StepKey,
        error: ResultRecord,
        by: Option<RunStep>,
    },
    #[fluent(transparent = warnings.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    Warned {
        step: StepKey,
        warnings: Vec<ResultRecord>,
        by: Option<RunStep>,
    },
    NeverBackedUp,
    /// macOS keeps rusticlone and rustic out of a source.
    #[fluent(exe = std::env::current_exe().map_or_else(|_| crate::NAME.into(), |e| e.display().to_string()))]
    NoDiskAccess {
        source: String,
    },
    Missing {
        repo: String,
        primary: bool,
    },
    Stale {
        repo: String,
        since: Since,
    },
    #[fluent(transparent = error)]
    InvalidSchedule {
        error: String,
    },
    /// Notifications cannot be sent.
    Undelivered {
        error: String,
    },
}

/// Since when an issue exists.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Since {
    Behind(Timestamp),
    LastBackup(Timestamp),
}

impl Since {
    /// As of `now`: `2 days behind`.
    fn text(self, now: Timestamp) -> String {
        match self {
            Self::Behind(t) => t!("since-behind", age = fmt::human_age(t, now)),
            Self::LastBackup(t) => t!("since-last-backup", age = fmt::human_age(t, now)),
        }
    }
}

impl Issue {
    /// A step's failed last attempt, which can be dismissed. A cancelled attempt is not an issue: whoever
    /// cancelled it knows, it waits for its next scheduled time, and what it left undone becomes stale
    /// meanwhile.
    fn failed(last: Option<&LastStep>, key: &StepKey) -> Option<Issue> {
        let last = last?;
        Some(Issue::Failed { step: key.clone(), error: last.last_error.clone()?, by: last.by.clone() })
    }

    /// Counts towards the collection's level, and is notified. A failing notification is neither: its
    /// notification would fail too.
    pub(crate) fn counts(&self) -> bool {
        !matches!(self, Self::Undelivered { .. })
    }

    /// The run and step that caused it.
    #[cfg(feature = "server")]
    pub(crate) fn by(&self) -> Option<&RunStep> {
        let (Self::Failed { by, .. } | Self::CopyFailed { by, .. } | Self::Warned { by, .. }) = self else {
            return None;
        };
        by.as_ref()
    }

    pub(crate) fn step(&self) -> Option<&StepKey> {
        let (Self::Failed { step, .. } | Self::CopyFailed { step, .. } | Self::Warned { step, .. }) = self else {
            return None;
        };
        Some(step)
    }

    /// Where to fix it.
    #[cfg(feature = "server")]
    pub(crate) fn fix_url(&self) -> Option<&'static str> {
        matches!(self, Self::NoDiskAccess { .. })
            .then_some("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
    }

    /// How long it has existed, shown after its text and kept current.
    pub(crate) fn since(&self) -> Option<Since> {
        let Self::Stale { since, .. } = self else { return None };
        Some(*since)
    }

    /// As of `now`: `Docs-usb is stale: 2 days behind`.
    pub(crate) fn text(&self, now: Timestamp) -> String {
        let Some(since) = self.since() else { return self.to_string() };
        t!("issue-since", issue = self.to_string(), since = since.text(now))
    }
}

impl Message for Issue {
    fn level(&self) -> Level {
        match self {
            Self::Unreadable { .. }
            | Self::CopyTargetUnusable(_)
            | Self::NoPassword { .. }
            | Self::SameRepository { .. }
            | Self::Failed { .. }
            | Self::Missing { primary: true, .. }
            | Self::InvalidSchedule { .. } => Level::Failure,
            Self::PrunesOnForget { .. }
            | Self::SettingIgnored { .. }
            | Self::CopyFailed { .. }
            | Self::Warned { .. }
            | Self::NeverBackedUp
            | Self::NoDiskAccess { .. }
            | Self::Stale { .. }
            | Self::Undelivered { .. }
            | Self::Missing { primary: false, .. } => Level::Warning,
        }
    }
}

/// When something last succeeded.
#[derive(Clone, Copy, Default, PartialEq)]
pub(crate) enum Last {
    At(Timestamp),
    #[default]
    Never,
    /// Unchanged sources made no snapshot, and no backup run is recorded yet.
    Unknown,
    /// Failed this many times in a row.
    Failing(u32),
}

impl Last {
    /// How [`Last::Never`] reads.
    pub(crate) fn never() -> String {
        t!("last-never")
    }

    /// `newer`: when it was done by other means, if after the step's last success. `failing`: whether a
    /// failure is reported, rather than pending.
    fn shown(last: Option<&LastStep>, newer: Option<Timestamp>, failing: &dyn Fn(&LastStep) -> bool) -> Self {
        match last {
            Some(t) if failing(t) => Self::Failing(t.failures),
            t => t.and_then(|t| t.last_ok).max(newer).map_or(Self::Never, Self::At),
        }
    }
}

impl CollectionOverview {
    /// The primary's last backup. For a target: its last copy, or its newest snapshot if copied by other
    /// means.
    pub(crate) fn backed_up(&self, r: &RepoOverview) -> Last {
        if r.name != self.primary.name {
            return r.copied;
        }
        match (self.last_backup, r.latest_snapshot) {
            (None, Some(_)) => Last::Unknown,
            (last, _) => last.map_or(Last::Never, Last::At),
        }
    }
}

#[derive(Default, Serialize)]
pub(crate) struct CollectionOverview {
    pub title: String,
    pub level: Level,
    #[serde(serialize_with = "crate::log::serialize_messages")]
    pub issues: Vec<Issue>,
    /// The last recorded backup run, or the primary's newest snapshot if newer. `None` with snapshots
    /// when unchanged sources make none and no backup run is recorded.
    pub last_backup: Option<Timestamp>,
    pub backup_schedule: Option<Schedule>,
    pub snapshot: Option<LastStep>,
    pub primary: RepoOverview,
    pub targets: Vec<RepoOverview>,
    /// In this process or another one.
    pub running: Vec<live::Running>,
}
