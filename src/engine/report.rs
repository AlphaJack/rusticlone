//! What the status says about each collection and repository: the report that `collections`, the web UI and
//! notifications show.

use jiff::Timestamp;
use serde::Serialize;

use super::Schedule;
use crate::request::StepKey;
use crate::result::Unusable;
use crate::{
    fmt,
    i18n::t,
    live,
    log::{Level, Message},
    rustic::Untested,
    settings::{self, Ignored},
    shape,
    state::{Lag, LastStep, ResultRecord, RunStep},
};

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
    pub(super) fn issues(
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
    pub(super) fn failed(last: Option<&LastStep>, key: &StepKey) -> Option<Issue> {
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
    pub(super) fn shown(
        last: Option<&LastStep>,
        newer: Option<Timestamp>,
        failing: &dyn Fn(&LastStep) -> bool,
    ) -> Self {
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
