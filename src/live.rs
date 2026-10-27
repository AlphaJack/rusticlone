//! The runs in progress in this process: their activity, their log, whether they should stop, and
//! who to notify when they change.

#![expect(clippy::disallowed_methods, reason = "records when an activity starts")]

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use jiff::Timestamp;
use parking_lot::Mutex;
use serde::Serialize;

use crate::{
    request::{RunKind, Trigger},
    result::{Doing, Status, StepResult},
    rustic::{Call, Origin, Progress, RunHandle},
    state::{Holds, RunLog, RunRecord, StepRecord},
};

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Activity {
    /// The index of the run's step that does it.
    pub step: Option<usize>,
    #[serde(serialize_with = "crate::log::serialize_message")]
    pub doing: Option<Doing>,
    /// The repository it reads or changes, and what it does: `copying`.
    pub on: Option<(String, String)>,
    pub since: Timestamp,
    pub progress: Option<Progress>,
}

#[derive(Serialize)]
pub(crate) struct Running {
    pub id: i64,
    pub kind: RunKind,
    pub label: String,
    pub trigger: Trigger,
    pub started: Timestamp,
    pub cancelling: bool,
    /// In another process, so its activities and cancellation are unknown here.
    pub elsewhere: bool,
    /// What each of its running steps does, by step.
    pub activities: Vec<Activity>,
    pub holds: Holds,
}

impl Running {
    /// A run in another process. [`Live::running`] is one in this process.
    pub(crate) fn elsewhere(run: &RunRecord) -> Self {
        Self {
            id: run.id,
            kind: run.kind,
            label: run.label.clone(),
            trigger: run.trigger,
            started: run.started,
            cancelling: false,
            elsewhere: true,
            activities: Vec::new(),
            holds: run.holds(),
        }
    }
}

/// Called with a collection whose runs, logs or progress changed.
type OnChange = Arc<dyn Fn(&str) + Send + Sync>;

pub(crate) struct Runs {
    live: Mutex<BTreeMap<(String, i64), Arc<Live>>>,
    /// What queued runs wait for before they start.
    waits: Mutex<BTreeMap<(String, i64), Doing>>,
    on_change: OnChange,
}

impl Runs {
    /// `on_change` is called with a collection whose runs, logs or progress changed.
    pub(crate) fn new(on_change: impl Fn(&str) + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self { live: Mutex::default(), waits: Mutex::default(), on_change: Arc::new(on_change) })
    }

    /// Every run, by collection, oldest first.
    pub(crate) fn all(&self) -> Vec<Arc<Live>> {
        self.live.lock().values().cloned().collect()
    }

    /// Oldest first.
    pub(crate) fn of(&self, collection: &str) -> Vec<Arc<Live>> {
        let runs = self.live.lock();
        runs.values().filter(|l| l.collection == collection).cloned().collect()
    }

    /// What the queued run waits for. `None`: it no longer waits.
    pub(crate) fn wait(&self, collection: &str, id: i64, doing: Option<Doing>) {
        let (mut waits, key) = (self.waits.lock(), (collection.to_owned(), id));
        if waits.get(&key) == doing.as_ref() {
            return;
        }
        match doing {
            Some(doing) => waits.insert(key, doing),
            None => waits.remove(&key),
        };
        drop(waits);
        (self.on_change)(collection);
    }

    pub(crate) fn waiting(&self, collection: &str, id: i64) -> Option<Doing> {
        self.waits.lock().get(&(collection.to_owned(), id)).cloned()
    }
}

/// A run in [`Runs`] while the guard lives.
pub(crate) struct Guard {
    pub live: Arc<Live>,
    runs: Arc<Runs>,
}

impl Guard {
    pub(crate) fn start(runs: &Arc<Runs>, log: RunLog) -> Self {
        let live = Arc::new(Live {
            collection: log.record.collection.clone(),
            id: log.record.id,
            activities: Mutex::default(),
            log: Mutex::new(log),
            cancel: AtomicBool::new(false),
            cancelled_steps: Mutex::default(),
            on_change: runs.on_change.clone(),
        });
        runs.live.lock().insert((live.collection.clone(), live.id), live.clone());
        live.changed();
        Self { live, runs: runs.clone() }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.runs.live.lock().remove(&(self.live.collection.clone(), self.live.id));
        self.live.changed();
    }
}

/// A run in progress. The rest is in its record.
pub(crate) struct Live {
    pub collection: String,
    pub id: i64,
    /// One per step that runs, by step: steps without conflicting locks run at the same time.
    activities: Mutex<Vec<Activity>>,
    log: Mutex<RunLog>,
    cancel: AtomicBool,
    /// Steps stopped from the web UI. The run goes on with those that do not need them.
    cancelled_steps: Mutex<BTreeSet<usize>>,
    on_change: OnChange,
}

impl Live {
    fn changed(&self) {
        (self.on_change)(&self.collection);
    }

    pub(crate) fn waited(&self, step: Option<usize>, secs: u64) {
        self.log.lock().waited(step, secs);
    }

    pub(crate) fn steps(&self, steps: &[StepRecord]) {
        self.log.lock().steps(steps);
        self.changed();
    }

    /// Its record so far: steps, and results no step produced.
    pub(crate) fn record(&self) -> RunRecord {
        self.log.lock().record.clone()
    }

    pub(crate) fn finish(&self, status: Status, results: &[StepResult], calls: Vec<Call>) -> anyhow::Result<()> {
        let finished = self.log.lock().finish(status, results, calls);
        self.changed();
        finished
    }

    pub(crate) fn activities(&self) -> Vec<Activity> {
        self.activities.lock().clone()
    }

    /// What `step` does. Doing the same again keeps when it started, as waiting for a lock does at every change
    /// of the state.
    pub(crate) fn set_activity(&self, step: Option<usize>, doing: Option<Doing>, on: Option<(String, String)>) {
        let mut activities = self.activities.lock();
        if activities.iter().any(|a| (a.step, &a.doing, &a.on) == (step, &doing, &on)) {
            return;
        }
        activities.retain(|a| a.step != step);
        activities.push(Activity { step, doing, on, since: Timestamp::now(), progress: None });
        activities.sort_by_key(|a| a.step);
        drop(activities);
        self.changed();
    }

    pub(crate) fn clear_activity(&self, step: Option<usize>) {
        self.activities.lock().retain(|a| a.step != step);
        self.changed();
    }

    pub(crate) fn set_holds(&self, step: Option<usize>, holds: Holds) {
        if self.log.lock().holds(step, holds) {
            self.changed();
        }
    }

    pub(crate) fn progress(&self, step: Option<usize>, progress: &Progress) {
        if let Some(a) = self.activities.lock().iter_mut().find(|a| a.step == step) {
            a.progress = Some(progress.clone());
        }
        self.changed();
    }

    #[cfg(feature = "server")]
    pub(crate) fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.changed();
        crate::clock::wake();
    }

    #[cfg(feature = "server")]
    pub(crate) fn cancel_step(&self, step: usize) {
        self.cancelled_steps.lock().insert(step);
        self.changed();
        crate::clock::wake();
    }

    pub(crate) fn trigger(&self) -> Trigger {
        self.log.lock().record.trigger
    }

    pub(crate) fn running(&self) -> Running {
        Running {
            cancelling: self.cancel.load(Ordering::Relaxed),
            elsewhere: false,
            activities: self.activities(),
            ..Running::elsewhere(&self.log.lock().record)
        }
    }
}

impl RunHandle for Live {
    fn log(&self, origin: Origin, line: &str) {
        self.log.lock().line(origin, line);
        self.changed();
    }

    fn called(&self, calls: &[Call]) {
        self.log.lock().calls(calls);
        self.changed();
    }

    fn cancelled(&self, step: Option<usize>) -> bool {
        self.cancel.load(Ordering::Relaxed) || step.is_some_and(|s| self.cancelled_steps.lock().contains(&s))
    }
}
