//! Run history and logs.

use askama::Template;
use jiff::{SignedDuration, Timestamp};

use super::{Age, Label, Route};
use crate::{
    engine::{Engine, Last},
    fmt,
    live::{Activity, Running},
    log::{Level, Tone},
    result::{Doing, Status},
    rustic::{Call, Ended, Origin, Progress},
    state::{Holds, LogLine, ResultRecord, RunRecord, StepRecord},
};

/// The Run details card of a collection: the newest run, or the one picked in its runs.
#[derive(Default)]
pub(super) struct LogView {
    /// The run's id. `None`: nothing to show.
    pub(super) run: Option<i64>,
    pub(super) collection: String,
    /// It runs here, so its running step can be stopped.
    pub(super) stoppable: bool,
    /// Results that no step produced.
    pub(super) results: Vec<(&'static str, String)>,
    /// Its planned steps, and how far each got.
    pub(super) steps: Vec<StepView>,
    pub(super) calls: Vec<CallView>,
    pub(super) lines: LogLines,
    /// The log offset where the lines end.
    at: u64,
    /// The page already shows these lines. New ones arrive as `log` events.
    pub(super) kept: bool,
}

/// A rustic command of the run, and the step that made it.
pub(super) struct CallView {
    pub(super) command: String,
    pub(super) duration: String,
    /// `exit 1`, or why it ended without an exit code.
    pub(super) exit: String,
    /// The class of its dot: whether it exited 0, or `running`.
    pub(super) class: &'static str,
    /// How long it has been running.
    pub(super) since: Option<Age>,
    pub(super) step: Option<usize>,
}

impl From<Call> for CallView {
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a float cast saturates")]
    fn from(c: Call) -> Self {
        let (command, duration) = (c.command(), fmt::human_duration(c.secs as u64));
        let tone = match c.exit.as_deref() {
            None => Tone::Running,
            Some("0") => Tone::Success,
            Some(_) => Tone::Failure,
        };
        let since = c.exit.is_none().then_some(Age::For(c.started));
        let exit = c.exit.map(|e| e.parse().map_or(e, |code| Ended::Exit { code }.to_string())).unwrap_or_default();
        let class = tone.into();
        Self { command, duration, exit, class, since, step: c.step }
    }
}

/// A step of the run, and its results.
pub(super) struct StepView {
    pub(super) label: String,
    /// Excluding the wait.
    pub(super) duration: String,
    /// How long it waited for resources another run held. Empty if it did not wait.
    pub(super) waited: String,
    /// How long it has been running.
    pub(super) since: Option<Age>,
    /// While it runs here.
    pub(super) progress: Option<ProgressView>,
    pub(super) holds: Holds,
    pub(super) status: String,
    pub(super) class: &'static str,
    pub(super) results: Vec<(&'static str, String)>,
}

/// `shown`: the log the page already shows.
pub(super) fn log_view(
    engine: &Engine,
    collection: &str,
    rows: &[RunRow],
    picked: Option<i64>,
    shown: Option<(i64, u64)>,
) -> LogView {
    // A queued run has no log yet: show it only when picked.
    let row = rows.iter().filter(|r| !r.waiting || picked.is_some()).find(|r| picked.is_none_or(|id| r.id == id));
    // Hidden: the Runs card says there is none.
    let Some(row) = row else {
        return LogView::default();
    };
    let tail = match shown {
        Some((run, at)) if run == row.id => (LogLines::default(), at, true),
        _ => {
            let (lines, at) = log_tail(engine, collection, row.id, None);
            (lines, at, false)
        }
    };
    LogView::build(row, engine.store.run(collection, row.id), tail)
}

impl LogView {
    /// The run whose log it shows, and the log offset where its lines end.
    pub(super) fn shown(&self) -> Option<(i64, u64)> {
        self.run.map(|run| (run, self.at))
    }

    /// Its rustic commands, one per line.
    pub(super) fn commands(&self) -> String {
        self.calls.iter().map(|c| c.command.as_str()).collect::<Vec<_>>().join("\n")
    }

    /// `tail`: the log lines, the offset where they end, and whether the page already has them.
    pub(super) fn build(row: &RunRow, record: Option<RunRecord>, tail: (LogLines, u64, bool)) -> Self {
        let (results, calls, steps) = record.map(|r| (r.results, r.calls, r.steps)).unwrap_or_default();
        let running = |s: &StepRecord| s.status == Status::Running && row.since.is_some();
        let step = |(i, s): (usize, &StepRecord)| StepView {
            label: s.label.clone(),
            duration: ran_for(s.started, s.finished, s.waited),
            waited: waited(s.waited),
            since: s.started.filter(|_| running(s)).map(Age::For),
            progress: row.progress.iter().find(|p| p.step == Some(i)).cloned(),
            holds: s.holds.clone(),
            status: s.status.to_string(),
            class: s.status.tone().into(),
            // Without the step's repository, which the row names.
            results: s.results.iter().map(|l| (l.level.into(), s.result(l))).collect(),
        };
        let (lines, at, kept) = tail;
        LogView {
            run: Some(row.id),
            collection: row.collection.clone(),
            stoppable: row.stoppable && row.since.is_some(),
            results: results.iter().map(|l| (l.level.into(), l.to_string())).collect(),
            steps: steps.iter().enumerate().map(step).collect(),
            calls: calls.into_iter().map(CallView::from).collect(),
            lines,
            at,
            kept,
        }
    }
}

/// A run's log from offset `from`, and the offset where it ends.
pub(super) fn log_tail(engine: &Engine, collection: &str, run: i64, from: Option<u64>) -> (LogLines, u64) {
    let (lines, end) = engine.store.run_log_tail(collection, run, from);
    (LogLines(lines.into_iter().map(LogLineView::from).collect()), end)
}

/// Log lines, which the page appends as they come.
#[derive(Template, Default)]
#[template(path = "log_lines.html")]
pub(super) struct LogLines(pub(super) Vec<LogLineView>);

/// A log line in local time: `11:03:42 rustic backup` in Berlin.
pub(super) struct LogLineView {
    pub(super) text: String,
    pub(super) origin: Origin,
}

impl From<LogLine> for LogLineView {
    fn from(LogLine { at, origin, text }: LogLine) -> Self {
        let text = at.map_or(text.clone(), |at| format!("{} {text}", fmt::local_time(at)));
        Self { text, origin }
    }
}

#[derive(Clone)]
pub(super) struct RunRow {
    pub(super) collection: String,
    /// What its collection is called.
    pub(super) title: String,
    pub(super) id: i64,
    /// The class of its status dot. `running` shows a spinner.
    pub(super) level: &'static str,
    pub(super) time: Age,
    /// "Copy to Docs-usb", "Check Docs".
    pub(super) label: String,
    pub(super) trigger: &'static str,
    pub(super) duration: String,
    /// How long it waited for resources another run held. Empty if it did not wait.
    pub(super) waited: String,
    /// How long it has been waiting, while it waits for resources.
    pub(super) waiting_since: Option<Age>,
    /// How long it has been running, without its waits. `None` while it waits.
    pub(super) since: Option<Age>,
    pub(super) summary: String,
    /// While it runs: what each running step does, or one view of the run when none does yet.
    pub(super) progress: Vec<ProgressView>,
    pub(super) holds: Holds,
    /// The page of the run that holds what it waits for.
    pub(super) holder: Option<String>,
    /// Waiting for an earlier run of the same request. It has no log yet.
    pub(super) waiting: bool,
    /// Its log is the one the page shows.
    pub(super) shown: bool,
    pub(super) stoppable: bool,
}

/// What a running run is doing, as far as this process knows.
#[derive(Clone)]
pub(super) struct ProgressView {
    /// The index of the step that does it.
    pub(super) step: Option<usize>,
    pub(super) doing: String,
    pub(super) percent: Option<u32>,
    pub(super) detail: String,
    pub(super) cancelling: bool,
}

impl RunRow {
    /// `live`: the run, if it still runs.
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a float cast saturates")]
    pub(super) fn new(collection: &str, title: &str, r: &RunRecord, live: Option<&Running>) -> Self {
        // The results with the worst level, without the repository the label already names.
        let lines: Vec<&ResultRecord> = r.all_results().collect();
        let worst = lines.iter().map(|l| l.level).max();
        let worst: Vec<String> =
            lines.iter().filter(|l| Some(l.level) == worst).map(|l| l.within(r.scope.as_slice())).collect();
        let summary = Some(worst.join("; ")).filter(|s| !s.is_empty());
        let duration = ran_for(Some(r.started), r.finished, r.waited());
        let waited = waited(r.waited());
        let (level, duration, summary) = match (r.status, live) {
            (Status::Queued, _) => (Tone::Running.into(), String::new(), Label::WaitingForTheRunning.to_string()),
            // The step it is at, whose progress the Steps card shows.
            (Status::Running, Some(_)) => {
                let at = r.steps.iter().enumerate().find(|(_, s)| s.status == Status::Running);
                (
                    Tone::Running.into(),
                    String::new(),
                    at.map(|(i, s)| format!("{}. {}", i + 1, s.label)).unwrap_or_default(),
                )
            }
            (Status::Running, None) => (Level::Warning.into(), String::new(), Label::Left.to_string()),
            // Stopped: its log shows how far it got.
            (status @ (Status::Cancelled | Status::Interrupted), _) => {
                (status.level().into(), duration, status.to_string())
            }
            (status, _) => (status.level().into(), duration, summary.unwrap_or_else(|| status.to_string())),
        };
        let progress = live.map_or_else(Vec::new, |l| {
            let view = |activity: Option<&Activity>| {
                let progress = activity.and_then(|a| a.progress.as_ref());
                ProgressView {
                    step: activity.and_then(|a| a.step),
                    doing: match activity {
                        Some(a) => a.doing.as_ref().map(ToString::to_string).unwrap_or_default(),
                        None if l.elsewhere => Label::Elsewhere { who: &l.trigger.who() }.to_string(),
                        None => String::new(),
                    },
                    percent: progress.map(|p| (p.percent_done * 100.0).round() as u32),
                    detail: progress.map(Progress::detail).unwrap_or_default(),
                    cancelling: l.cancelling,
                }
            };
            if l.activities.is_empty() {
                vec![view(None)]
            } else {
                l.activities.iter().map(|a| view(Some(a))).collect()
            }
        });
        let queued = r.status == Status::Queued;
        // Waits before the current one count as waiting, not running.
        let before = SignedDuration::from_secs(i64::try_from(r.waited()).unwrap_or_default());
        // Waiting while every step that runs waits for resources.
        let activities = live.map(|l| l.activities.as_slice()).unwrap_or_default();
        let waits = |a: &Activity| matches!(a.doing, Some(Doing::WaitingFor { .. }));
        let waiting = !activities.is_empty() && activities.iter().all(waits);
        let since = activities.iter().map(|a| a.since).min().filter(|_| waiting);
        let waiting_since = since.map(|since| Age::For(since - before));
        RunRow {
            collection: collection.to_owned(),
            title: title.to_owned(),
            id: r.id,
            level,
            time: if queued { Age::Queued } else { Age::Last(Last::At(r.started)) },
            label: fmt::capitalized(&r.label),
            trigger: r.trigger.into(),
            duration,
            waited,
            since: live.filter(|_| waiting_since.is_none()).map(|_| Age::For(r.started + before)),
            waiting_since,
            summary,
            progress,
            // Not those of a process that stopped before the run ended.
            holds: live.map(|l| l.holds.clone()).unwrap_or_default(),
            holder: None,
            waiting: queued,
            shown: false,
            // A queued run can be removed from the queue in any process.
            stoppable: queued || live.is_some_and(|l| !l.elsewhere && !l.cancelling),
        }
    }
}

/// How long it ran, without the wait. Empty until it ends.
fn ran_for(started: Option<Timestamp>, finished: Option<Timestamp>, waited: u64) -> String {
    let (Some(start), Some(end)) = (started, finished) else { return String::new() };
    let secs = u64::try_from(end.duration_since(start).as_secs()).unwrap_or_default();
    fmt::human_duration(secs.saturating_sub(waited).max(1))
}

/// Empty if it did not wait.
fn waited(secs: u64) -> String {
    if secs > 0 { fmt::human_duration(secs) } else { String::new() }
}

/// Runs waiting for an earlier run of the same request, in any process, the next to run on top; then
/// the other runs, newest first: the newest `limit`. A run that is running here or queued can be stopped.
pub(super) fn collection_run_rows(engine: &Engine, collection: &str, limit: usize) -> Vec<RunRow> {
    let (runs, running) = (engine.store.runs(collection, limit), engine.running(collection));
    let (runs, title) =
        (runs.iter().filter(|r| r.status != Status::Queued || engine.store.alive(r)), engine.title(collection));
    let mut rows: Vec<RunRow> =
        runs.map(|r| RunRow::new(collection, &title, r, running.iter().find(|l| l.id == r.id))).collect();
    rows.sort_by_key(|r| std::cmp::Reverse((r.waiting, r.id)));
    let queue = engine.store.queue(Some(collection));
    for row in &mut rows {
        if let Some(lock) = &row.holds.waiting_for {
            row.holder = engine.store.holding(lock).first().map(|h| Route::Run(&h.collection, h.id).to_string());
        }
        let queued = queue.iter().find(|q| row.waiting && q.record.id == row.id);
        if let Some((own, Some(request))) = queued.map(|q| (&q.record, &q.request)) {
            row.summary = engine.queued_wait(&queue, own, request).to_string();
        }
    }
    rows
}
