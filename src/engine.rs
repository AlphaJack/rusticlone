//! Back up into the collection's repository, copy to its targets, forget only what every target has, prune,
//! check and restore.

mod backup;
mod collection;
mod copy;
mod load;
mod locks;
mod maintain;
mod plan;
mod record;
mod restore;
mod schedule;
mod snapshots;
mod status;
#[cfg(test)]
mod tests;

#[cfg(feature = "server")]
pub(crate) use collection::title;
pub(crate) use collection::{Blocked, Collection};
pub(crate) use load::Named;
use locks::Use;
use plan::{follows, plan};
use record::behind;
pub(crate) use restore::parse_at;
pub(crate) use schedule::Schedule;
use snapshots::{each, newest, newest_by_source, pick, shrunk};
pub(crate) use status::{CollectionOverview, Last, RepoOverview, StatusReport};
#[cfg(feature = "server")]
pub(crate) use status::{Issue, RepoStatus, Unconfigured};

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::Result;
use jiff::{Timestamp, tz::TimeZone};
use parking_lot::Mutex;

use crate::request::{Request, Step, StepKey};
use crate::result::Doing;
use crate::{
    clock,
    live::{Guard, Live, Runs},
    log::{Level, Logged, Message as _},
    profile::RusticProfile,
    request::Trigger,
    result::{self, RunResults, Status, StepResult},
    rustic::{Ctx, Failure, Rustic, Snapshot, Stop},
    settings,
    state::{Event, Read, ResultRecord, RunLog, RunRecord, RunStep, StepRecord, Store},
};

#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "engine-error")]
pub(crate) enum Error {
    /// `known`: the collections there are, to pick from.
    NoCollection {
        name: String,
        known: String,
    },
    /// Where a first-time user starts.
    NoCollections {
        dirs: String,
    },
    NotASource {
        path: String,
    },
    NoRepository {
        name: String,
    },
    NotARepository {
        name: String,
        collection: String,
    },
    NotOfProfile {
        name: String,
        collection: String,
    },
    BackupOfRepository,
    #[fluent(transparent)]
    Blocked(Blocked),
    NoSubset {
        repo: String,
    },
    #[fluent(key = settings::CHECK_SUBSETS.key)]
    NoSubsets {
        value: String,
    },
    FromItself {
        repo: String,
    },
    #[fluent(sources = not_backed_up(sources))]
    NotBackedUp {
        sources: Vec<(String, String)>,
    },
    #[fluent(paths = format!("{paths:?}"), expected = format!("{expected:?}"))]
    Unexpected {
        snapshot: String,
        paths: Vec<String>,
        expected: BTreeSet<String>,
    },
    UnlistedAfter(#[source] Source),
    Filtered {
        snapshot: String,
    },
    PrimaryUnlisted {
        repo: String,
        error: String,
    },
    NotRecreated {
        repo: String,
        from: String,
        error: Failure,
    },
    FolderAway {
        repo: String,
        folder: String,
    },
    NotCopiedInto {
        from: String,
        repo: String,
        error: Failure,
    },
    NotListed {
        repo: String,
    },
    NotCopiedFrom {
        repo: String,
    },
    Unavailable {
        repo: String,
        error: String,
    },
    Unreachable(#[source] Source),
    NotCopied {
        failed: usize,
        of: usize,
        error: Failure,
    },
    NoSnapshotOfThem {
        repo: String,
    },
    /// No repository could be listed, which says why each failed and which profiles name their places.
    Unreadable {
        collection: String,
        repos: String,
        files: String,
    },
    NoMatchingSnapshot {
        repo: String,
    },
    #[fluent(transparent = path)]
    NotRestored {
        path: String,
        source: Source,
    },
    /// Some of its paths failed, which the run reported.
    PartlyRestored {
        repo: String,
    },
    NoTime {
        time: String,
        source: jiff::Error,
    },
}

type Source = Box<dyn std::error::Error + Send + Sync>;

/// `/home/me/docs (cause)`, for each source, as rustic said.
fn not_backed_up(sources: &[(String, String)]) -> String {
    sources.iter().map(|(path, cause)| format!("{path} ({cause})")).collect::<Vec<_>>().join("; ")
}

fn queued(step: &Step, needs: Vec<usize>) -> StepRecord {
    StepRecord { repo: step.on().map(|(repo, _)| repo.to_owned()), ..StepRecord::queued(step.label(), needs) }
}

/// One invocation: who started it, and the repositories pruned and checked so far, as `prune:Docs`. Profiles
/// can share targets.
pub(crate) struct Batch {
    pub trigger: Trigger,
    done: Mutex<BTreeSet<String>>,
    /// A run started, instead of being skipped because it runs elsewhere.
    pub ran: AtomicBool,
}

impl Batch {
    pub(crate) fn new(trigger: Trigger) -> Self {
        Self { trigger, done: Mutex::default(), ran: AtomicBool::new(false) }
    }
}

/// One run of a collection: its report, its log, the locks its rustic calls inherit, and its listings. A
/// repository is listed again only after the run changed it. Each step that runs beside others has its own
/// [`Run::for_step`], which shares the rest.
pub(crate) struct Run<'a> {
    pub collection: &'a Collection<'a>,
    /// `None` for listings outside runs.
    pub live: Option<Arc<Live>>,
    pub cx: Ctx,
    shared: Arc<Shared>,
}

/// What a run's steps share.
#[derive(Default)]
struct Shared {
    /// Every result, with the step that produced it: `None` for the run's own.
    results: Mutex<Vec<(Option<usize>, StepResult)>>,
    /// Its planned steps. [`Ctx::step`] is the index of the current one.
    steps: Mutex<Vec<Step>>,
    listed: Mutex<BTreeMap<String, Vec<Snapshot>>>,
    /// A copy step ran. If the run stops, the remaining copies count as cancelled.
    copying: AtomicBool,
    /// The targets copied to, with their newest snapshot per group.
    copied: Mutex<BTreeMap<String, BTreeMap<String, Timestamp>>>,
}

impl<'a> Run<'a> {
    pub(crate) fn new(collection: &'a Collection<'a>, live: Option<Arc<Live>>) -> Self {
        Self { collection, cx: live.clone().map_or_else(Ctx::default, |l| Ctx::new(l)), live, shared: Arc::default() }
    }

    /// The run as step `i` sees it: its own context, which the step's log lines, calls and locks belong to.
    fn for_step(&self, i: usize) -> Self {
        let cx = self.cx.for_step(i);
        cx.hold(&self.cx.held());
        Self { collection: self.collection, live: self.live.clone(), cx, shared: self.shared.clone() }
    }

    fn push(&self, result: StepResult) {
        self.cx.log(None, &result.to_string());
        self.shared.results.lock().push((self.cx.step(), result));
    }

    /// Pushes `result` unless the run has it already, as when steps running at once report the same.
    fn push_once(&self, result: StepResult) {
        let mut results = self.shared.results.lock();
        if !results.iter().any(|(_, r)| *r == result) {
            self.cx.log(None, &result.to_string());
            results.push((self.cx.step(), result));
        }
    }

    /// How many failures the run reported so far.
    fn failures(&self) -> usize {
        self.shared.results.lock().iter().filter(|(_, r)| r.level() == Level::Failure).count()
    }

    /// Whether any step reported a result that `f` matches.
    fn reported(&self, f: impl Fn(&StepResult) -> bool) -> bool {
        self.shared.results.lock().iter().any(|(_, r)| f(r))
    }

    /// The results of step `step`, or the run's own with `None`.
    fn results_of(&self, step: Option<usize>) -> Vec<StepResult> {
        self.shared.results.lock().iter().filter(|(s, _)| *s == step).map(|(_, r)| r.clone()).collect()
    }

    /// Lists the repository again when it is next needed.
    fn changed(&self, repo: &RusticProfile) {
        self.shared.listed.lock().remove(&repo.name);
    }

    fn step(&self) -> Option<Step> {
        self.shared.steps.lock().get(self.cx.step()?).cloned()
    }

    /// The state's key of the current step.
    fn key(&self) -> Option<StepKey> {
        self.step()?.key(self.collection.name())
    }

    fn label(&self) -> String {
        self.step().map(|s| s.label()).unwrap_or_default()
    }

    /// The current step failed, or was stopped.
    fn failed(&self, e: &anyhow::Error) -> StepResult {
        StepResult::Failed { step: self.label(), error: format!("{e:#}"), stopped: self.cx.stopping() }
    }

    /// Shows what the current step does now. `on`: it works on its repository, rather than waiting.
    fn activity(&self, doing: Option<Doing>, on: bool) {
        let on = self.step().filter(|_| on).and_then(|s| s.on().map(|(repo, verb)| (repo.to_owned(), verb)));
        if let Some(l) = &self.live {
            l.set_activity(self.cx.step(), doing, on);
        }
    }

    fn idle(&self) {
        if let Some(l) = &self.live {
            l.clear_activity(self.cx.step());
        }
    }

    /// The run and step that make an attempt now, if a run.
    fn by(&self, step: Option<usize>) -> Option<RunStep> {
        self.live.as_ref().map(|l| RunStep {
            collection: l.collection.clone(),
            run: l.id,
            step,
            scheduled: l.trigger() == Trigger::Schedule,
        })
    }

    fn level(&self) -> Level {
        result::worst(&self.all_results()).unwrap_or_default()
    }

    fn all_results(&self) -> Vec<StepResult> {
        self.shared.results.lock().iter().map(|(_, r)| r.clone()).collect()
    }

    fn run_results(&self) -> RunResults {
        RunResults { collection: self.collection.title.to_owned(), results: self.all_results() }
    }
}

/// What [`Engine::ask`] did with a request.
#[expect(clippy::large_enum_variant, reason = "one per request of an invocation")]
pub(crate) enum Asked {
    Queued(RunLog),
    /// The same request was queued already: [`Engine::follow`] waits for its run.
    Joined(RunRecord),
}

impl Asked {
    pub(crate) fn record(&self) -> &RunRecord {
        match self {
            Self::Queued(queued) => &queued.record,
            Self::Joined(run) => run,
        }
    }
}

pub(crate) struct Engine {
    pub rustic: Rustic,
    pub store: Store,
    pub runs: Arc<Runs>,
    pub rustic_profiles: BTreeMap<String, RusticProfile>,
    pub broken: Vec<(String, String)>,
    /// Every profile in the scanned directories, which a collection's title must not start.
    profiles: Vec<String>,
    /// What the command line names. Nothing named: every collection.
    pub named: Named,
    tz: TimeZone,
}

impl Engine {
    /// A run of `request`. From a command or the web UI, it is queued and visible until it runs, unless
    /// the same request that [joins](Request::joins) is queued already, in any process; a check takes its
    /// data subset now, so that it shows while queued. A scheduled run is not queued: it is skipped while
    /// the same request runs.
    pub(crate) fn ask(&self, collection: &Collection<'_>, request: &Request, trigger: Trigger) -> Result<Asked> {
        // A dry run records nothing, so it has no place in the queue.
        if trigger == Trigger::Schedule || self.store.is_dry_run() {
            return Ok(Asked::Queued(RunLog::new(&self.store, collection.name(), request.clone(), trigger)));
        }
        let request = self.claim(collection, request).unwrap_or_else(|_| request.clone());
        let queued = self.store.queue(Some(collection.name())).into_iter().find(|q| {
            q.record.status == Status::Queued && q.request.as_ref() == Some(&request) && self.store.alive(&q.record)
        });
        match queued {
            Some(q) if request.joins() => Ok(Asked::Joined(q.record)),
            _ => RunLog::queued(&self.store, collection.name(), request, trigger).map(Asked::Queued),
        }
    }

    /// Runs what [`Self::ask`] returned, under its request's lock. A queued run waits for earlier runs of
    /// the same request, and for the holder of that lock.
    pub(crate) fn execute(&self, collection: &Collection<'_>, mut run: RunLog, batch: &Batch) -> RunResults {
        let (mut r, cx) = (RunResults::new(collection.title), Ctx::default());
        // A target of several collections is maintained once per invocation.
        let maintained = run.request.maintained().into_iter().map(str::to_owned).collect::<Vec<_>>();
        if !maintained.is_empty() {
            let kind: &str = run.request.kind().into();
            let mut done = batch.done.lock();
            let fresh: Vec<&String> = maintained.iter().filter(|repo| done.insert(format!("{kind}:{repo}"))).collect();
            drop(done);
            self.unread(collection, &run.request.retain(|repo| !fresh.iter().any(|f| *f == repo)));
            if fresh.is_empty() {
                return r;
            }
            run.request = run.request.retain(|repo| fresh.iter().any(|f| *f == repo));
        }
        let request = run.request.clone();
        let queued = (run.record.status == Status::Queued).then_some(&run.record);
        match self.lock_for(collection, &request, &cx, &mut r, queued) {
            Some(_lock) => r.results.extend(self.run_request(collection, run, batch, &cx).0.results),
            None => self.unread(collection, &request),
        }
        r
    }

    /// Runs each due request, unless it runs elsewhere. A cancelled run ends the round.
    pub(crate) fn schedule(&self, collection: &Collection<'_>, batch: &Batch) -> RunResults {
        let mut r = RunResults::new(collection.title);
        let mut done = Vec::new();
        while !clock::interrupted() {
            let next = match self.due_next(collection, &done) {
                Ok(Some(next)) => next,
                Ok(None) => break,
                Err(e) => {
                    r.push(StepResult::Error { error: format!("{e:#}") });
                    break;
                }
            };
            // A dry run says what is due, and runs nothing.
            if self.store.is_dry_run() {
                r.push(StepResult::WouldRun { run: next.label(collection.name()) });
                done.push(next);
                continue;
            }
            let cx = Ctx::default();
            let lock = self.lock_for(collection, &next, &cx, &mut r, None);
            let status = lock.map(|_lock| {
                let log = RunLog::new(&self.store, collection.name(), next.clone(), batch.trigger);
                let (report, status) = self.run_request(collection, log, batch, &cx);
                r.results.extend(report.results);
                status
            });
            done.push(next);
            if status == Some(Status::Cancelled) {
                break;
            }
        }
        r
    }

    /// Runs one request and records it with its log. Its rustic calls also inherit the locks in `held`.
    fn run_request(
        &self,
        collection: &Collection<'_>,
        mut log: RunLog,
        batch: &Batch,
        held: &Ctx,
    ) -> (RunResults, Status) {
        batch.ran.store(true, Ordering::Relaxed);
        let claimed = self.claim(collection, &log.request);
        if let Ok(request) = &claimed {
            log.set_request(request.clone());
        }
        log.start();
        let guard = Guard::start(&self.runs, log);
        let run = Run::new(collection, Some(guard.live.clone()));
        run.cx.hold(&held.held());
        let request = claimed.as_ref().ok().cloned();
        let steps = match claimed.and_then(|request| plan(&request, collection)) {
            Ok(steps) => steps,
            Err(e) => {
                run.push(StepResult::Error { error: format!("{e:#}") });
                Vec::new()
            }
        };
        self.run_steps(&run, &guard.live, &steps, request.as_ref());
        let status = Status::ended(run.level(), run.cx.stop());
        if let Err(e) = guard.live.finish(status, &run.results_of(None), run.cx.calls()) {
            crate::log::event(Some(collection.title), &Logged::RunNotSaved { error: format!("{e:#}") });
        }
        (run.run_results(), status)
    }

    /// Runs the planned steps: each once the steps before it ended, those it needs and the one it follows.
    /// Steps that are ready run at the same time, and wait for each other only for the locks they share.
    fn run_steps(&self, run: &Run<'_>, live: &Live, steps: &[(Step, Vec<usize>)], request: Option<&Request>) {
        *run.shared.steps.lock() = steps.iter().map(|(step, _)| step.clone()).collect();
        // Every step is known from the start, and updated as it ends.
        let records: Vec<StepRecord> = steps.iter().cloned().map(|(step, needs)| queued(&step, needs)).collect();
        live.steps(&records);
        let records = Mutex::new(records);
        // Records step `i` as ended with `status`, with the results it reported.
        let end = |i: usize, status: Status, started: Timestamp| {
            let (step, needs) = &steps[i];
            let mut records = records.lock();
            let results = run.results_of(Some(i)).iter().map(ResultRecord::from).collect();
            // Only a step that ran has a duration.
            let ran = records[i].started.is_some();
            records[i] = StepRecord {
                status,
                started: ran.then_some(started),
                finished: ran.then(|| self.store.now()),
                results,
                ..queued(step, needs.clone())
            };
            live.steps(&records);
        };
        std::thread::scope(|scope| {
            let (tx, rx) = std::sync::mpsc::channel();
            let (mut begun, mut ended) = (vec![false; steps.len()], vec![false; steps.len()]);
            let mut running = 0;
            loop {
                for (i, (step, needs)) in steps.iter().enumerate() {
                    if begun[i] || !needs.iter().copied().chain(follows(request, i)).all(|n| ended[n]) {
                        continue;
                    }
                    begun[i] = true;
                    let (started, at_step) = (self.store.now(), run.for_step(i));
                    let unmet: Vec<String> = {
                        let records = records.lock();
                        let unmet = needs.iter().filter_map(|n| records.get(*n)).filter(|r| !r.status.done());
                        unmet.map(|r| r.label.clone()).collect()
                    };
                    if run.cx.stopping() {
                        self.cancelled(run, i, step, started);
                        end(i, Status::Cancelled, started);
                        ended[i] = true;
                    } else if !unmet.is_empty() {
                        at_step.push(StepResult::Unmet { step: step.label(), needs: unmet });
                        end(i, Status::Skipped, started);
                        ended[i] = true;
                    } else {
                        {
                            let mut records = records.lock();
                            (records[i].status, records[i].started) = (Status::Running, Some(started));
                            live.steps(&records);
                        }
                        running += 1;
                        let tx = tx.clone();
                        scope.spawn(move || {
                            self.run_step(&at_step, step);
                            _ = tx.send((i, started));
                        });
                    }
                }
                if running == 0 {
                    break;
                }
                let Ok((i, started)) = rx.recv() else { break };
                running -= 1;
                end(i, Status::of(&run.results_of(Some(i))), started);
                ended[i] = true;
            }
        });
    }

    /// Step `i`, which did not start because the run stopped. Cancelled copies wait for the next backup.
    fn cancelled(&self, run: &Run<'_>, i: usize, step: &Step, started: Timestamp) {
        let collection = run.collection;
        if let Step::Copy { .. } = step
            && run.shared.copying.load(Ordering::Relaxed)
            && run.cx.cancelled()
            && let Some(key) = step.key(collection.name())
        {
            let (at, by) = (self.store.now(), run.by(Some(i)));
            let result = StepResult::Failed { step: step.label(), error: Stop::Cancelled.to_string(), stopped: true };
            self.emit(collection.name(), &[Event::Attempted { key, started, at, result: (&result).into(), by }]);
        }
        if let Step::Check { repo, reading } = step {
            self.check_read(collection, repo, *reading, Read::Not);
        }
    }

    fn run_step(&self, run: &Run<'_>, step: &Step) {
        let collection = run.collection;
        match step {
            Step::Snapshot { .. } => self.backup(run),
            Step::Copy { to } => match collection.repo(to) {
                Some(t) => self.copy_step(run, t),
                None => run.push(StepResult::CopyTargetUnusable(self.unusable(to))),
            },
            Step::Forget { repo: name } if *name == collection.name() => self.forget_primary(run),
            Step::Forget { repo: name } => {
                if let Some(t) = collection.repo(name) {
                    self.forget(run, t, &[]);
                }
            }
            Step::Prune { repo: name } | Step::Check { repo: name, .. } => {
                if let Some(r) = collection.repo(name) {
                    self.maintain(run, r, step);
                }
            }
            Step::Restore { options } => {
                if let Err(e) = self.restore_inner(run, options) {
                    run.push(run.failed(&e));
                }
            }
            Step::TestRestore { from, tested } => {
                if let Err(e) = self.test_restore(run, from.as_deref(), tested) {
                    run.push(run.failed(&e));
                }
            }
            Step::Recreate { repo: name, from, requested } => {
                let Some(r) = collection.repo(name) else { return };
                let from = from.as_deref().and_then(|f| collection.repo(f));
                if self.list(run, r).is_ok() {
                    if *requested {
                        run.push(StepResult::Listable { repo: r.name.clone() });
                    }
                } else if self.store.is_dry_run() {
                    run.push(StepResult::WouldRecreate { repo: r.name.clone() });
                } else if r.name == collection.name() {
                    self.copy_back(run, from);
                } else {
                    self.recreate_target(run, r, from);
                }
            }
        }
    }
}
