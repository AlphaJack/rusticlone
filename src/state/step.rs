//! The last step of each kind on each repository: when it was done, and how it went.

use std::collections::BTreeSet;

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

use super::ResultRecord;
use crate::result::Status;
use crate::result::StepResult;

/// The delay before the first retry, doubled at each failure.
const FIRST_RETRY: SignedDuration = SignedDuration::from_mins(5);
const LAST_RETRY: SignedDuration = SignedDuration::from_hours(6);

/// After `failures` failures in a row, the last at `last`, when to retry: [`FIRST_RETRY`] later, doubling each
/// time up to [`LAST_RETRY`].
pub(super) fn retry_at(last: Timestamp, failures: u32) -> Option<Timestamp> {
    let backoff = FIRST_RETRY.saturating_mul(2_i32.saturating_pow(failures.saturating_sub(1))).min(LAST_RETRY);
    last.checked_add(backoff).ok()
}

/// The run, and its step, that made an attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStep {
    pub collection: String,
    pub run: i64,
    pub step: Option<usize>,
    /// Started by the schedule, which retries it after a failure.
    #[serde(default)]
    pub scheduled: bool,
}

/// A step's result as an [`Event`](super::Event) carries it: what is shown, and how the step ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub status: Status,
    pub record: ResultRecord,
}

impl From<&StepResult> for Outcome {
    fn from(result: &StepResult) -> Self {
        Self { status: Status::of(std::slice::from_ref(result)), record: result.into() }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LastStep {
    pub last_ok: Option<Timestamp>,
    pub last_attempt: Option<Timestamp>,
    pub failures: u32,
    pub last_error: Option<ResultRecord>,
    pub note: Option<ResultRecord>,
    /// The last attempt was cancelled: neither a success nor a failure.
    pub cancelled: bool,
    /// When the last success started. What it produced is newer.
    pub ok_started: Option<Timestamp>,
    /// What the last success left out: items a backup could not read.
    pub warnings: Vec<ResultRecord>,
    /// What made the last attempt, if a run.
    #[serde(default)]
    pub by: Option<RunStep>,
}

impl LastStep {
    /// A cancelled attempt is neither a success nor a failure: the step waits for its next scheduled time.
    pub(super) fn record(&mut self, at: Timestamp, outcome: &Outcome) {
        self.last_attempt = Some(at);
        self.cancelled = outcome.status == Status::Cancelled;
        match outcome.status {
            Status::Succeeded | Status::SucceededWithWarnings => {
                self.last_ok = Some(at);
                self.failures = 0;
                self.last_error = None;
                self.warnings.clear();
                self.note = Some(outcome.record.clone());
            }
            Status::Failed => {
                self.failures += 1;
                self.last_error = Some(outcome.record.clone());
            }
            Status::Queued | Status::Running | Status::Skipped | Status::Cancelled | Status::Interrupted => {}
        }
    }

    pub fn retry_at(&self) -> Option<Timestamp> {
        retry_at(self.last_attempt.filter(|_| self.failures > 0)?, self.failures)
    }

    /// Until when a scheduled first failure is not reported yet: its retry may well succeed, as after a
    /// resume whose network is not up yet. Whoever started a run by hand sees it fail at once.
    pub fn pending_until(&self) -> Option<Timestamp> {
        self.retry_at().filter(|_| self.failures == 1 && self.by.as_ref().is_some_and(|b| b.scheduled))
    }

    /// Failed, and reported: again, or not retried in time. `busy`: a run of its collection, which may be
    /// the retry, is in progress.
    pub fn failing(&self, now: Timestamp, busy: bool) -> bool {
        self.failures > 0 && !self.pending_until().is_some_and(|until| busy || now < until)
    }
}

/// A repository's progress through its data subsets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Checks {
    /// The next subset in the cycle.
    pub next: u32,
    pub taken: u32,
    /// Taken but not read (failed, stopped or never run). Taken again before the cycle continues.
    pub owed: BTreeSet<u32>,
    /// Subsets whose last read failed. Checks fail until each one passes.
    pub failed: BTreeSet<u32>,
}

impl Checks {
    /// The subset the next check reads.
    pub fn upcoming(&self) -> u32 {
        self.owed.first().copied().unwrap_or(self.next.max(1))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lag {
    pub checked_at: Timestamp,
    /// The time of the primary's oldest snapshot that the target lacks.
    pub behind_since: Option<Timestamp>,
    /// Whether the target could be listed. If not, `behind_since` is an estimate.
    pub listed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ended(result: &StepResult) -> Outcome {
        result.into()
    }

    #[test]
    fn backoff_grows_and_caps() {
        let t0: Timestamp = "2026-10-14T00:00:00Z".parse().unwrap();
        let mut key = LastStep::default();
        assert_eq!(key.retry_at(), None);
        key.record(t0, &ended(&StepResult::Error { error: "repository unreachable".into() }));
        assert_eq!(key.retry_at(), Some(t0 + SignedDuration::from_mins(5)));
        for _ in 0..20 {
            key.record(t0, &ended(&StepResult::Error { error: "repository unreachable".into() }));
        }
        assert_eq!(key.retry_at(), Some(t0 + SignedDuration::from_hours(6)));
        key.record(t0, &ended(&StepResult::RunningAlready));
        assert_eq!((key.failures, key.retry_at()), (0, None));
    }

    #[test]
    fn a_scheduled_first_failure_is_reported_once_its_retry_is_late() {
        let t0: Timestamp = "2026-10-14T00:00:00Z".parse().unwrap();
        let mut key = LastStep::default();
        key.record(t0, &ended(&StepResult::Error { error: "repository unreachable".into() }));
        assert!(key.failing(t0, false), "a run started by hand fails at once");
        key.by = Some(RunStep { collection: "Docs".into(), run: 1, step: None, scheduled: true });
        let retry = key.retry_at().unwrap();
        assert!(!key.failing(t0, false) && !key.failing(retry, true) && key.failing(retry, false));
        key.record(retry, &ended(&StepResult::Error { error: "repository unreachable".into() }));
        assert!(key.failing(retry, true));
    }

    #[test]
    fn a_cancelled_attempt_is_neither_success_nor_failure() {
        let t0: Timestamp = "2026-10-14T00:00:00Z".parse().unwrap();
        let mut key = LastStep::default();
        key.record(t0, &ended(&StepResult::HeldBack { repo: "Docs".into(), paths: Vec::new() }));
        assert!(key.cancelled && key.last_attempt == Some(t0));
        assert!(key.failures == 0 && key.last_ok.is_none() && key.last_error.is_none());
        assert_eq!(key.retry_at(), None);
        key.record(t0, &ended(&StepResult::RunningAlready));
        assert!(!key.cancelled);
    }
}
