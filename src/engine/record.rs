//! What runs report: progress while it happens, and each step's outcome, stored in the state.

use anyhow::Result;
use jiff::Timestamp;

use super::{Collection, Engine, Run};
use crate::result::Doing;
use crate::{
    log::Logged,
    profile::RusticProfile,
    result::StepResult,
    rustic::Progress,
    state::{Event, Ledger},
};

impl Run<'_> {
    /// Runs a long rustic command of the current step, reporting its progress and showing it as the
    /// collection's activity. `doing`: what the step's label does not say.
    pub(super) fn tracked<T>(&self, doing: Option<Doing>, f: impl FnOnce(&mut dyn FnMut(&Progress)) -> T) -> T {
        let text =
            [self.label()].into_iter().chain(doing.as_ref().map(ToString::to_string)).collect::<Vec<_>>().join(" ");
        self.activity(doing, true);
        let mut report = |progress: &Progress| {
            crate::log::progress(self.collection.title, &text, &progress.summary(), progress.percent_done);
            if let Some(l) = &self.live {
                l.progress(self.cx.step(), progress);
            }
        };
        let out = f(&mut report);
        self.idle();
        out
    }
}

impl Engine {
    /// Reports and records the current step's outcome.
    pub(super) fn step(&self, run: &Run<'_>, started: Timestamp, result: Result<StepResult>) {
        let result = result.unwrap_or_else(|e| run.failed(&e));
        run.push(result.clone());
        self.record(run, started, &result);
    }

    /// Records the current step's result. A step stopped by a shutdown is not recorded: it is still
    /// due, and did not fail.
    pub(super) fn record(&self, run: &Run<'_>, started: Timestamp, result: &StepResult) {
        let Some(key) = run.key().filter(|_| !result.stopped() || run.cx.cancelled()) else { return };
        let (at, by) = (self.store.now(), run.by(run.cx.step()));
        self.emit(run.collection.name(), &[Event::Attempted { key, started, at, result: result.into(), by }]);
    }

    /// Failures to store events are only logged: the repositories are the source of truth, and the
    /// state is rebuilt from them.
    pub(super) fn emit(&self, collection: &str, events: &[Event]) {
        if let Err(e) = self.store.apply(events) {
            crate::log::event(Some(collection), &Logged::StateNotKept { error: format!("{e:#}") });
        }
    }

    pub(super) fn note_latest(&self, repo: &str, newest: Option<Timestamp>) {
        if let Some(at) = newest {
            self.emit(repo, &[Event::Newest { repo: repo.to_owned(), at }]);
        }
    }
}

/// Whether a target lacks snapshots of the primary.
pub(super) fn behind(state: &Ledger, collection: &Collection<'_>, repo: &RusticProfile) -> bool {
    state.lag(collection.name(), &repo.name).is_some_and(|l| l.behind_since.is_some())
}
