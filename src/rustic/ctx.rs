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

#[cfg(test)]
mod tests {
    use super::*;

    /// A run that records what it is told.
    #[derive(Default)]
    struct Told {
        lines: Mutex<Vec<(Origin, String)>>,
        calls: Mutex<Vec<Call>>,
    }

    impl RunHandle for Told {
        fn log(&self, origin: Origin, line: &str) {
            self.lines.lock().push((origin, line.to_owned()));
        }

        fn cancelled(&self, _: Option<usize>) -> bool {
            false
        }

        fn called(&self, calls: &[Call]) {
            calls.clone_into(&mut self.calls.lock());
        }
    }

    fn call(args: &str) -> Call {
        let started = "2026-10-16T12:00:00Z".parse().unwrap();
        Call { step: None, args: args.into(), bin: None, dir: None, started, secs: 0.0, exit: None }
    }

    /// Calls and lines belong to the step the context is at. Each call is told to the run as it starts and ends.
    #[test]
    fn calls_and_lines_belong_to_the_current_step() {
        let told = Arc::new(Told::default());
        let cx = Ctx::new(told.clone());
        assert!(format!("{cx:?}").starts_with("Ctx { in_run: true, step: "), "{cx:?}");
        cx.at_step(Some(2));
        cx.log(Some(0), "reading");
        assert_eq!(*told.lines.lock(), [(Origin { step: Some(2), call: Some(0) }, "reading".to_owned())]);
        assert_eq!([cx.call(call("backup")), cx.call(call("forget"))], [0, 1]);
        cx.ended(0, 2.5, "0".into());
        let calls = told.calls.lock().clone();
        let ended: Vec<_> = calls.iter().map(|c| (c.step, c.secs, c.exit.clone())).collect();
        assert_eq!(ended, [(Some(2), 2.5, Some("0".into())), (Some(2), 0.0, None)]);
    }

    /// Released locks are no longer held. The others stay.
    #[test]
    fn released_locks_are_no_longer_held() {
        let cx = Ctx::default();
        cx.hold(&[3, 4, 5]);
        cx.release(&[4]);
        assert_eq!(cx.held(), [3, 5]);
    }
}
