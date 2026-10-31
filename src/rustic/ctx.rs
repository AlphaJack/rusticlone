//! The context of rustic calls.

use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use super::Call;
use crate::clock;

/// The run that rustic calls belong to.
pub trait RunHandle: Send + Sync {
    fn log(&self, origin: Origin, line: &str);
    /// The run, or `step` of it, was cancelled.
    fn cancelled(&self, step: Option<usize>) -> bool;
    /// Its calls so far, the last ones possibly still running.
    fn called(&self, calls: &[Call]);
}

/// Where a log line comes from: the index of the run's step, and of its rustic call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    pub step: Option<usize>,
    pub call: Option<usize>,
}

/// Why a run stops early.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "lowercase")]
pub enum Stop {
    /// From the web UI.
    Cancelled,
    /// The process is stopping: a signal, or a shutdown.
    Interrupted,
}

/// What rustic calls belong to: the run that receives their log and can stop them (none outside
/// runs), the locks they inherit, the record of the calls, and the step they belong to.
#[derive(Default)]
pub struct Ctx {
    run: Option<Arc<dyn RunHandle>>,
    /// File descriptors of the held locks.
    held: Mutex<Vec<i32>>,
    /// The run's calls, in the order they started, whichever step made them.
    calls: Arc<Mutex<Vec<Call>>>,
    step: Mutex<Option<usize>>,
}

impl std::fmt::Debug for Ctx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ctx").field("in_run", &self.run.is_some()).field("step", &self.step).finish_non_exhaustive()
    }
}

impl Ctx {
    /// The context of the run.
    pub fn new(run: Arc<dyn RunHandle>) -> Self {
        Self { run: Some(run), ..Self::default() }
    }

    /// The context of one of the run's steps, which runs beside the others: its own locks, the run's calls.
    pub fn for_step(&self, step: usize) -> Self {
        Self { run: self.run.clone(), held: Mutex::default(), calls: self.calls.clone(), step: Mutex::new(Some(step)) }
    }

    /// Logs a line of the current step, and of `call` if a rustic call wrote it.
    pub fn log(&self, call: Option<usize>, line: &str) {
        if let Some(run) = &self.run {
            run.log(Origin { step: *self.step.lock(), call }, line);
        }
    }

    pub fn step(&self) -> Option<usize> {
        *self.step.lock()
    }

    /// The step that the next calls and lines belong to.
    pub fn at_step(&self, step: Option<usize>) {
        *self.step.lock() = step;
    }

    pub fn cancelled(&self) -> bool {
        self.run.as_ref().is_some_and(|run| run.cancelled(*self.step.lock()))
    }

    /// Cancelled, or the process is stopping: no new step may start.
    pub fn stop(&self) -> Option<Stop> {
        if self.cancelled() { Some(Stop::Cancelled) } else { clock::interrupted().then_some(Stop::Interrupted) }
    }

    pub fn stopping(&self) -> bool {
        self.stop().is_some()
    }

    pub fn held(&self) -> Vec<i32> {
        self.held.lock().clone()
    }

    pub(crate) fn hold(&self, fds: &[i32]) {
        self.held.lock().extend(fds);
    }

    pub(crate) fn release(&self, fds: &[i32]) {
        self.held.lock().retain(|fd| !fds.contains(fd));
    }

    /// Records a call as it starts, in the current step, and returns its index.
    pub fn call(&self, call: Call) -> usize {
        let mut calls = self.calls.lock();
        calls.push(Call { step: *self.step.lock(), ..call });
        self.tell(&calls);
        calls.len() - 1
    }

    pub fn ended(&self, call: usize, secs: f64, exit: String) {
        let mut calls = self.calls.lock();
        if let Some(call) = calls.get_mut(call) {
            (call.secs, call.exit) = (secs, Some(exit));
        }
        self.tell(&calls);
    }

    fn tell(&self, calls: &[Call]) {
        if let Some(run) = &self.run {
            run.called(calls);
        }
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().clone()
    }
}
