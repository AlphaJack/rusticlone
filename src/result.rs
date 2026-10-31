//! What users read: the results of steps, what runs are doing, and why they wait.

use std::path::PathBuf;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::{
    fmt,
    log::{self, Level, Message, Tone},
    request::RunKind,
    rustic::{Pruned, Reading, RestorePlan, Stop},
};
use crate::{i18n::t, request::Trigger};

/// How far a run, or one of its steps, got. A run that its process left queued or running keeps that
/// status. It is stale once the process no longer holds the run's lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, strum::IntoStaticStr, crate::i18n::Fluent)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum Status {
    /// A run waiting for an earlier run of the same request, or a step for the steps before it.
    Queued,
    /// Started and not ended.
    Running,
    /// Done without warnings.
    Succeeded,
    /// Done, with warnings.
    SucceededWithWarnings,
    /// Ended with an error.
    Failed,
    /// A step with nothing to do, or a step it needs did not do its job.
    Skipped,
    /// Stopped from the web UI. A step: the run stopped before or during it.
    Cancelled,
    /// A run stopped with the process: a signal, or a shutdown.
    Interrupted,
}

impl From<Stop> for Status {
    fn from(stop: Stop) -> Self {
        match stop {
            Stop::Cancelled => Self::Cancelled,
            Stop::Interrupted => Self::Interrupted,
        }
    }
}

impl From<Level> for Status {
    fn from(level: Level) -> Self {
        match level {
            Level::Success => Self::Succeeded,
            Level::Warning => Self::SucceededWithWarnings,
            Level::Failure => Self::Failed,
        }
    }
}

impl Status {
    /// A run's, by its worst result, unless it was stopped.
    pub fn ended(level: Level, stop: Option<Stop>) -> Self {
        stop.map_or(level.into(), Self::from)
    }

    /// A step's: cancelled if a result says so, else by its worst result. `Skipped` if it reported nothing.
    pub fn of(results: &[StepResult]) -> Self {
        match worst(results) {
            _ if results.iter().any(StepResult::stopped) => Self::Cancelled,
            // Nothing done, or nothing to do.
            None => Self::Skipped,
            _ if results.iter().all(StepResult::skipped) => Self::Skipped,
            Some(level) => level.into(),
        }
    }

    /// Whether it may still change: queued or running.
    pub fn unfinished(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }

    /// It did its job.
    pub fn done(self) -> bool {
        matches!(self, Self::Succeeded | Self::SucceededWithWarnings)
    }

    /// A run's: a stop is a warning.
    pub fn level(self) -> Level {
        match self {
            Self::Queued | Self::Running | Self::Succeeded | Self::Skipped => Level::Success,
            Self::SucceededWithWarnings | Self::Cancelled | Self::Interrupted => Level::Warning,
            Self::Failed => Level::Failure,
        }
    }

    /// A step's: what did not run is muted.
    pub fn tone(self) -> Tone {
        match self {
            Self::Queued | Self::Skipped | Self::Cancelled | Self::Interrupted => Tone::Muted,
            Self::Running => Tone::Running,
            Self::Succeeded | Self::SucceededWithWarnings | Self::Failed => self.level().into(),
        }
    }
}

/// What a step did, or why it could not. Stored in the run's record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum StepResult {
    /// `snapshots`: short ids. Empty when nothing changed.
    BackedUp {
        repo: String,
        snapshots: Vec<String>,
        new_repository: bool,
    },
    Copied {
        target: String,
        snapshots: u64,
    },
    InSync {
        target: String,
    },
    /// Expired snapshots kept: `held` until copied, `shrunk` because the newest one is almost empty.
    Forgot {
        repo: String,
        snapshots: u64,
        held: usize,
        shrunk: usize,
    },
    Pruned {
        repo: String,
        freed: Option<Pruned>,
    },
    Verified {
        repo: String,
        reading: Reading,
    },
    /// The files and bytes the snapshots would hold.
    WouldBackUp {
        repo: String,
        files: u64,
        bytes: u64,
    },
    /// `create`: the target does not exist yet.
    WouldCopy {
        target: String,
        snapshots: u64,
        create: bool,
    },
    WouldForget {
        repo: String,
        snapshots: u64,
    },
    WouldPrune {
        repo: String,
        freed: Option<Pruned>,
    },
    WouldCheck {
        repo: String,
        reading: Reading,
    },
    WouldRecreate {
        repo: String,
    },
    /// A first backup would create the primary. Rustic plans a backup only into an existing one.
    WouldCreatePrimary {
        repo: String,
    },
    /// A run that is due.
    WouldRun {
        run: String,
    },
    /// `due` ran nothing: the run due next and when, unless nothing is scheduled.
    NothingDue {
        next: Option<(String, Timestamp)>,
    },
    PrimaryCreated {
        repo: String,
        from: String,
    },
    Recreated {
        repo: String,
        from: String,
        snapshots: u64,
    },
    /// Re-created empty. The primary's snapshots follow with the next copy.
    TargetRecreated {
        target: String,
    },
    PrimaryRecreatedEmpty {
        repo: String,
    },
    /// Asked to be re-created, but it can be listed.
    Listable {
        repo: String,
    },
    /// The bytes read from a snapshot, and what restoring each of its paths would do.
    TestRestored {
        repo: String,
        snapshot: String,
        bytes: u64,
        plans: Vec<(String, Option<RestorePlan>)>,
    },
    Restored {
        path: String,
        repo: String,
        snapshot: String,
        time: Timestamp,
        to: Option<PathBuf>,
    },
    /// A dry run with restore choices: what would be restored, as the snapshot's summary counts it.
    WouldRestore {
        path: String,
        repo: String,
        snapshot: String,
        files: u64,
        bytes: u64,
        to: Option<PathBuf>,
    },
    /// The backup state now matches the restored snapshots.
    BackupRestored {
        from: String,
    },
    LeftAsItIs {
        path: String,
    },
    RunningAlready,
    /// The same request was queued already: the results of its run, `label`, are this one's.
    Joined {
        label: String,
        trigger: Trigger,
        pid: u32,
        level: Level,
    },
    /// Sources are absent or empty, but the repository has this computer's snapshots of them.
    HeldBack {
        repo: String,
        paths: Vec<String>,
    },
    /// `items`: the first few paths, with the reason. `count`: all of them.
    Skipped {
        items: Vec<(String, String)>,
        count: usize,
    },
    /// The primary lacks data that the previous snapshot needs, so the backup read every file again.
    ParentUnreadable {
        repo: String,
        path: String,
        why: String,
    },
    /// The newest snapshot of `paths` holds `holds` bytes; the previous one held `held`.
    Shrunk {
        paths: Vec<String>,
        holds: u64,
        held: Option<u64>,
    },
    PrunePostponed {
        repo: String,
    },
    /// It has no `[forget]` policy, so a prune has nothing to remove.
    KeepsEverything {
        repo: String,
    },
    Unavailable {
        repo: String,
        error: String,
    },
    SomeSources {
        found: usize,
        of: usize,
    },
    Unmounted {
        path: String,
    },
    /// Stdin, or a remote source: restored only into a folder.
    NoPlace {
        source: String,
    },
    /// Its backup hooks may mount it, and a restore runs none: restored only into a folder.
    Hooked {
        source: String,
    },
    NotRecreated {
        repo: String,
        from: String,
        error: String,
    },
    /// A step did not run, because the steps it needs did not succeed.
    Unmet {
        step: String,
        needs: Vec<String>,
    },
    Dequeued {
        error: String,
    },
    /// `step`: its label. `stopped`: cancelled or interrupted, which is not a failure.
    Failed {
        step: String,
        error: String,
        stopped: bool,
    },
    PrimaryUnavailable {
        repo: String,
        error: String,
        stopped: bool,
    },
    CopyTargetUnusable(Unusable),
    NoTargetListed,
    MissingSkipped {
        run: RunKind,
        repo: String,
    },
    Error {
        error: String,
    },
}

impl StepResult {
    /// Its step had nothing to do, by design.
    pub(crate) fn skipped(&self) -> bool {
        matches!(self, Self::KeepsEverything { .. })
    }

    /// Its step was cancelled or interrupted, or held back: neither a success nor a failure.
    pub(crate) fn stopped(&self) -> bool {
        matches!(
            self,
            Self::Failed { stopped: true, .. } | Self::PrimaryUnavailable { stopped: true, .. } | Self::HeldBack { .. }
        )
    }
}

impl Message for StepResult {
    fn level(&self) -> Level {
        match self {
            Self::Failed { stopped: true, .. }
            | Self::PrimaryUnavailable { stopped: true, .. }
            | Self::HeldBack { .. }
            | Self::Skipped { .. }
            | Self::ParentUnreadable { .. }
            | Self::Shrunk { .. }
            | Self::PrunePostponed { .. }
            | Self::Unavailable { .. }
            | Self::SomeSources { .. }
            | Self::Unmounted { .. }
            | Self::NoPlace { .. }
            | Self::Hooked { .. }
            | Self::NotRecreated { .. }
            | Self::Unmet { .. }
            | Self::Dequeued { .. } => Level::Warning,
            Self::Failed { .. }
            | Self::PrimaryUnavailable { .. }
            | Self::CopyTargetUnusable(_)
            | Self::NoTargetListed
            | Self::MissingSkipped { .. }
            | Self::Error { .. } => Level::Failure,
            Self::BackedUp { .. }
            | Self::Copied { .. }
            | Self::InSync { .. }
            | Self::Forgot { .. }
            | Self::Pruned { .. }
            | Self::Verified { .. }
            | Self::WouldBackUp { .. }
            | Self::WouldCopy { .. }
            | Self::WouldForget { .. }
            | Self::WouldPrune { .. }
            | Self::WouldCheck { .. }
            | Self::WouldRecreate { .. }
            | Self::WouldCreatePrimary { .. }
            | Self::WouldRun { .. }
            | Self::NothingDue { .. }
            | Self::PrimaryCreated { .. }
            | Self::Recreated { .. }
            | Self::TargetRecreated { .. }
            | Self::PrimaryRecreatedEmpty { .. }
            | Self::Listable { .. }
            | Self::TestRestored { .. }
            | Self::Restored { .. }
            | Self::WouldRestore { .. }
            | Self::BackupRestored { .. }
            | Self::LeftAsItIs { .. }
            | Self::KeepsEverything { .. }
            | Self::RunningAlready => Level::Success,
            Self::Joined { level, .. } => *level,
        }
    }
}

impl StepResult {
    /// The repository, target, path or step it concerns, which its text starts with.
    pub(crate) fn subject(&self) -> Option<&str> {
        match self {
            Self::BackedUp { repo: subject, .. }
            | Self::Forgot { repo: subject, .. }
            | Self::Pruned { repo: subject, .. }
            | Self::Verified { repo: subject, .. }
            | Self::WouldBackUp { repo: subject, .. }
            | Self::WouldForget { repo: subject, .. }
            | Self::WouldPrune { repo: subject, .. }
            | Self::WouldCheck { repo: subject, .. }
            | Self::WouldRecreate { repo: subject }
            | Self::WouldCreatePrimary { repo: subject }
            | Self::PrimaryCreated { repo: subject, .. }
            | Self::Recreated { repo: subject, .. }
            | Self::Listable { repo: subject }
            | Self::PrimaryRecreatedEmpty { repo: subject }
            | Self::HeldBack { repo: subject, .. }
            | Self::ParentUnreadable { repo: subject, .. }
            | Self::PrunePostponed { repo: subject }
            | Self::KeepsEverything { repo: subject }
            | Self::Unavailable { repo: subject, .. }
            | Self::PrimaryUnavailable { repo: subject, .. }
            | Self::NotRecreated { repo: subject, .. }
            | Self::Copied { target: subject, .. }
            | Self::InSync { target: subject }
            | Self::WouldCopy { target: subject, .. }
            | Self::TargetRecreated { target: subject }
            | Self::Restored { path: subject, .. }
            | Self::WouldRestore { path: subject, .. }
            | Self::LeftAsItIs { path: subject }
            | Self::Unmounted { path: subject }
            | Self::NoPlace { source: subject }
            | Self::Hooked { source: subject }
            | Self::Failed { step: subject, .. }
            | Self::Unmet { step: subject, .. } => Some(subject),
            Self::WouldRun { .. }
            | Self::NothingDue { .. }
            | Self::TestRestored { .. }
            | Self::BackupRestored { .. }
            | Self::RunningAlready
            | Self::Joined { .. }
            | Self::Skipped { .. }
            | Self::Shrunk { .. }
            | Self::SomeSources { .. }
            | Self::CopyTargetUnusable(_)
            | Self::NoTargetListed
            | Self::MissingSkipped { .. }
            | Self::Dequeued { .. }
            | Self::Error { .. } => None,
        }
    }
}

/// `subject: detail`.
impl std::fmt::Display for StepResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.subject() {
            Some(subject) => write!(f, "{subject}: {}", Detail(self)),
            None => write!(f, "{}", Detail(self)),
        }
    }
}

/// A result's text after its [`StepResult::subject`].
pub(crate) struct Detail<'a>(pub(crate) &'a StepResult);

impl std::fmt::Display for Detail<'_> {
    #[expect(clippy::too_many_lines, reason = "one arm per kind of result")]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&match self.0 {
            StepResult::BackedUp { snapshots, new_repository: true, .. } => {
                t!("result-backed-up-new", ids = snapshots.join(", "))
            }
            StepResult::BackedUp { snapshots, .. } => {
                t!("result-backed-up", count = snapshots.len(), ids = snapshots.join(", "))
            }
            StepResult::Copied { snapshots, .. } => t!("result-copied", snapshots = *snapshots),
            StepResult::InSync { .. } => t!("result-in-sync"),
            StepResult::Forgot { snapshots, held, shrunk, .. } => {
                let kept = [
                    (*held > 0).then(|| t!("result-forgot-held", held = *held)),
                    (*shrunk > 0).then(|| t!("result-forgot-shrunk", shrunk = *shrunk)),
                ];
                let forgot = std::iter::once(t!("result-forgot", snapshots = *snapshots));
                forgot.chain(kept.into_iter().flatten()).collect::<Vec<_>>().join(", ")
            }
            StepResult::Pruned { freed: None, .. } => t!("result-pruned"),
            StepResult::Pruned { freed: Some(Pruned { removed, remaining, unused }), .. } => {
                t!("result-pruned-freed", removed, remaining, unused)
            }
            StepResult::Verified { reading, .. } => t!("result-verified", what = reading.what()),
            StepResult::WouldBackUp { files, bytes, .. } => {
                t!("result-would-back-up", files = *files, size = fmt::human_size(*bytes))
            }
            StepResult::WouldCopy { snapshots, create: true, .. } => {
                t!("result-would-copy-created", snapshots = *snapshots)
            }
            StepResult::WouldCopy { snapshots, .. } => t!("result-would-copy", snapshots = *snapshots),
            StepResult::WouldForget { snapshots, .. } => t!("result-would-forget", snapshots = *snapshots),
            StepResult::WouldPrune { freed: None, .. } => t!("result-would-prune"),
            StepResult::WouldPrune { freed: Some(Pruned { removed, remaining, unused }), .. } => {
                t!("result-would-prune-freed", removed, remaining, unused)
            }
            StepResult::WouldCheck { reading, .. } => t!("result-would-check", what = reading.what()),
            StepResult::WouldRecreate { .. } => t!("result-would-recreate"),
            StepResult::WouldCreatePrimary { .. } => t!("result-would-create-primary"),
            StepResult::WouldRun { run } => t!("result-would-run", run),
            StepResult::NothingDue { next: Some((run, at)) } => t!("result-nothing-due", run, at = fmt::local(*at)),
            StepResult::NothingDue { next: None } => t!("result-nothing-scheduled"),
            StepResult::PrimaryCreated { from, .. } => t!("result-primary-created", from),
            StepResult::Recreated { from, snapshots, .. } => t!("result-recreated", from, snapshots = *snapshots),
            StepResult::TargetRecreated { .. } => t!("result-target-recreated"),
            StepResult::Listable { .. } => t!("result-listable"),
            StepResult::PrimaryRecreatedEmpty { .. } => t!("result-primary-recreated-empty"),
            StepResult::TestRestored { repo, snapshot, bytes, plans } => {
                let plans = plans.iter().map(|(path, plan)| match plan {
                    // `keep`: files that already match the snapshot.
                    Some(p) => t!(
                        "result-test-restored-plan",
                        path,
                        create = p.create,
                        replace = p.replace,
                        keep = p.keep,
                        additional = p.additional
                    ),
                    None => t!("result-test-restored-no-counts", path),
                });
                let read = t!("result-test-restored", repo, snapshot, size = fmt::human_size(*bytes));
                std::iter::once(read).chain(plans).collect::<Vec<_>>().join("; ")
            }
            StepResult::Restored { repo, snapshot, time, to: Some(to), .. } => {
                t!("result-restored-to", repo, snapshot, time = fmt::local(*time), to = to.display())
            }
            StepResult::Restored { repo, snapshot, time, to: None, .. } => {
                t!("result-restored", repo, snapshot, time = fmt::local(*time))
            }
            StepResult::WouldRestore { repo, snapshot, files, bytes, to: Some(to), .. } => t!(
                "result-would-restore-to",
                repo,
                snapshot,
                files = *files,
                size = fmt::human_size(*bytes),
                to = to.display()
            ),
            StepResult::WouldRestore { repo, snapshot, files, bytes, to: None, .. } => {
                t!("result-would-restore", repo, snapshot, files = *files, size = fmt::human_size(*bytes))
            }
            StepResult::BackupRestored { from } => t!("result-backup-restored", from),
            StepResult::LeftAsItIs { .. } => t!("result-left-as-it-is"),
            StepResult::RunningAlready => t!("result-running-already"),
            StepResult::Joined { label, trigger, pid, .. } => {
                t!("result-joined", label, pid = *pid, who = trigger.who())
            }
            StepResult::HeldBack { paths, .. } => t!("result-held-back", paths = paths.join(", ")),
            StepResult::Skipped { items, count } => {
                let more = *count > items.len();
                let shown: Vec<String> =
                    items.iter().map(|(path, error)| t!("result-skipped-item", path, error)).collect();
                let items = shown.join(", ");
                if more {
                    t!("result-skipped-more", count = *count, items)
                } else {
                    t!("result-skipped", count = *count, items)
                }
            }
            StepResult::ParentUnreadable { path, why, .. } => t!("result-parent-unreadable", path, error = why),
            StepResult::Shrunk { paths, holds, held } => t!(
                "result-shrunk",
                paths = paths.join(", "),
                now = fmt::human_size(*holds),
                before = held.map(fmt::human_size).unwrap_or_default()
            ),
            StepResult::PrunePostponed { .. } => t!("result-prune-postponed"),
            StepResult::KeepsEverything { .. } => t!("result-keeps-everything"),
            StepResult::Unavailable { error, .. } | StepResult::PrimaryUnavailable { error, .. } => {
                t!("result-unavailable", error)
            }
            StepResult::SomeSources { found, of } => t!("result-some-sources", found = *found, of = *of),
            StepResult::Unmounted { .. } => t!("result-unmounted"),
            StepResult::NoPlace { .. } => t!("result-no-place"),
            StepResult::Hooked { .. } => t!("result-hooked"),
            StepResult::NotRecreated { from, error, .. } => t!("result-not-recreated", from, error),
            StepResult::Unmet { needs, .. } => t!("result-unmet", needs = needs.join(", ")),
            StepResult::CopyTargetUnusable(unusable) => unusable.to_string(),
            StepResult::NoTargetListed => t!("result-no-target-listed"),
            StepResult::MissingSkipped { run, repo } => t!("result-missing-skipped", run = <&str>::from(run), repo),
            StepResult::Failed { error, .. } | StepResult::Dequeued { error } | StepResult::Error { error } => {
                error.clone()
            }
        })
    }
}

/// A copy target in `[copy] targets` that cannot be used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, crate::i18n::Fluent)]
#[fluent(prefix = "result-unusable")]
pub struct Unusable {
    pub target: String,
    /// Its profile exists but cannot be read.
    pub broken: bool,
}

/// What a step does now that its label does not say. `{:#}` makes what it waits for bold.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum Doing {
    Snapshot {
        snapshot: String,
    },
    Path {
        path: String,
    },
    CopyFrom {
        from: String,
    },
    /// Waiting for a repository whose listings may lag its writes.
    WaitingForListing {
        repo: String,
    },
    /// `lock`: the one it waits for first, shown while no run is known to hold it.
    WaitingFor {
        lock: String,
        by: Vec<Holder>,
    },
    Listing {
        repo: String,
    },
}

impl std::fmt::Display for Doing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Snapshot { snapshot } => f.write_str(snapshot),
            Self::Path { path } => f.write_str(path),
            Self::CopyFrom { from } => f.write_str(&t!("doing-copy-from", from)),
            Self::WaitingForListing { repo } => f.write_str(&t!("doing-waiting-for-listing", repo)),
            Self::WaitingFor { lock, by } if by.is_empty() => f.write_str(&t!("doing-waiting-for", by = lock)),
            Self::WaitingFor { by, .. } => {
                let by: Vec<String> =
                    by.iter().map(|h| if f.alternate() { format!("{h:#}") } else { h.to_string() }).collect();
                f.write_str(&t!("doing-waiting-for", by = by.join(", ")))
            }
            Self::Listing { repo } => f.write_str(&t!("doing-listing", repo)),
        }
    }
}

log::level!(Success: Doing);

/// A process whose runs hold locks that another run waits for, named as the runs table shows them:
/// `web 4247 (repo:Docs-remote, download)`, or for a run ahead of a queued one, its request and status:
/// `cli 55378 (backup, running)`. `{:#}`: the locks in bold.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Holder {
    pub trigger: Trigger,
    pub pid: u32,
    pub locks: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
}

impl std::fmt::Display for Holder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let locks: Vec<String> =
            self.locks.iter().map(|l| if f.alternate() { log::bold(l) } else { l.clone() }).collect();
        let status = self.status.map(|s| format!(", {s}")).unwrap_or_default();
        write!(f, "{} {} ({}{status})", <&str>::from(self.trigger), self.pid, locks.join(", "))
    }
}

pub(crate) fn worst<M: Message>(results: &[M]) -> Option<Level> {
    results.iter().map(Message::level).max()
}

pub(crate) struct RunResults {
    pub collection: String,
    pub results: Vec<StepResult>,
}

impl RunResults {
    pub(crate) fn new(collection: &str) -> Self {
        Self { collection: collection.to_owned(), results: Vec::new() }
    }

    pub(crate) fn push(&mut self, result: StepResult) {
        self.results.push(result);
    }

    pub(crate) fn level(&self) -> Level {
        worst(&self.results).unwrap_or_default()
    }

    pub(crate) fn print(&self) {
        for result in &self.results {
            match result.subject() == Some(self.collection.as_str()) {
                // `Docs: backed up`, not `Docs: Docs: backed up`.
                true => log::event(Some(&self.collection), &Unnamed(result)),
                false => log::event(Some(&self.collection), result),
            }
        }
    }
}

/// A result without its subject, after a prefix that names it already. Recorded as the result.
#[derive(Serialize)]
#[serde(transparent)]
struct Unnamed<'a>(&'a StepResult);

impl std::fmt::Display for Unnamed<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Detail(self.0).fmt(f)
    }
}

impl Message for Unnamed<'_> {
    fn level(&self) -> Level {
        self.0.level()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn how_runs_end() {
        assert_eq!(Status::ended(Level::Success, None), Status::Succeeded);
        assert_eq!(Status::ended(Level::Warning, None), Status::SucceededWithWarnings);
        assert_eq!(Status::ended(Level::Failure, None), Status::Failed);
        // Stopped: whatever the results say.
        assert_eq!(Status::ended(Level::Failure, Some(Stop::Cancelled)), Status::Cancelled);
    }
}
