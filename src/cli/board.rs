//! In a terminal, each run of this process as its plan of steps, redrawn as it goes. A run that ends
//! stays printed, and its results are left out of the report that follows.

use super::tree::branch;
use std::{
    collections::BTreeMap,
    iter,
    sync::{Arc, OnceLock, Weak},
    time::Duration,
};

use parking_lot::Mutex;
use serde::Serialize;

use super::tree::{Tree, tree};
use crate::{
    clock,
    engine::Engine,
    live::{Live, Runs},
    log::{self, Message, Tone},
    result::{RunResults, Status},
    rustic::Progress,
    state::{ResultRecord, RunRecord, StepRecord},
};

#[cfg(any(feature = "desktop", feature = "self-update"))]
mod command;

#[cfg(any(feature = "desktop", feature = "self-update"))]
pub(super) use command::{command, step};

/// A run as last drawn.
struct Drawn {
    lines: Vec<String>,
    /// The results the lines show.
    shown: Vec<ResultRecord>,
}

/// A step outside runs, as a run of only that step.
struct Outside {
    collection: String,
    label: String,
    step: String,
}

struct Board {
    runs: OnceLock<Weak<Runs>>,
    /// What each collection is called, by key.
    titles: OnceLock<BTreeMap<String, String>>,
    drawn: Mutex<BTreeMap<(String, i64), Drawn>>,
    /// Runs this process queued and has not started, with their labels.
    queued: Mutex<BTreeMap<(String, i64), String>>,
    /// Results that ended runs showed.
    shown: Mutex<Vec<ResultRecord>>,
    /// The step this process does outside runs.
    doing: Mutex<Option<Outside>>,
    /// Lines of steps outside runs that ended, to print with the next drawing.
    ended: Mutex<Vec<String>>,
}

static BOARD: Board = Board {
    runs: OnceLock::new(),
    titles: OnceLock::new(),
    drawn: Mutex::new(BTreeMap::new()),
    queued: Mutex::new(BTreeMap::new()),
    shown: Mutex::new(Vec::new()),
    doing: Mutex::new(None),
    ended: Mutex::new(Vec::new()),
};

/// The id of steps outside runs, which runs never have.
const OUTSIDE: i64 = -1;

/// The runs of this process, drawn in a terminal.
pub(super) fn runs() -> Arc<Runs> {
    if !log::is_live() {
        return Runs::new(|_| {});
    }
    let runs = Runs::new(|_| draw());
    let weak = Arc::downgrade(&runs);
    _ = BOARD.runs.set(weak.clone());
    std::thread::spawn(move || spin(&weak));
    runs
}

/// How often a running step's spinner turns.
const SPIN: Duration = Duration::from_millis(250);

/// Turns the spinners of running steps while the runs exist.
#[expect(clippy::disallowed_methods, reason = "a spinner turns with time, which no event tells")]
fn spin(runs: &Weak<Runs>) {
    while runs.strong_count() > 0 && !clock::interrupted() {
        clock::sleep(SPIN, &clock::interrupted);
        log::turn();
        draw();
    }
}

/// Names the runs by their collection's title.
pub(super) fn titles(engine: &Engine) {
    _ = BOARD.titles.set(engine.collections().iter().map(|j| (j.name().to_owned(), j.title.to_owned())).collect());
}

/// What the collection `name` is called.
fn title(name: &str) -> &str {
    BOARD.titles.get().and_then(|t| t.get(name)).map_or(name, String::as_str)
}

/// Shows `runs` as queued until they start.
pub(super) fn queued<'a>(runs: impl IntoIterator<Item = &'a RunRecord>) {
    if BOARD.runs.get().is_some() {
        BOARD.queued.lock().extend(runs.into_iter().map(|r| ((r.collection.clone(), r.id), r.label.clone())));
        draw();
    }
}

/// Stops showing `run` as queued: it ended, perhaps without starting.
pub(super) fn ended(run: &RunRecord) {
    if BOARD.queued.lock().remove(&(run.collection.clone(), run.id)).is_some() {
        draw();
    }
}

/// Leaves out of `report` the results that the runs showed.
pub(super) fn leave_out(report: &mut RunResults) {
    let mut shown = BOARD.shown.lock();
    report.results.retain(|result| {
        let i = shown.iter().position(|s| *s == ResultRecord::from(result));
        i.map(|i| shown.remove(i)).is_none()
    });
}

/// Prints `report`, in a terminal as a run's steps below `Docs: heading`.
pub(super) fn print(report: &RunResults, heading: &str) {
    if BOARD.runs.get().is_none() || report.results.is_empty() {
        return report.print();
    }
    let results: Vec<&dyn Message> = report.results.iter().map(|r| -> &dyn Message { r }).collect();
    section(&report.collection, heading, &results);
}

/// Does `step` in each of the collections `names` with `f`, one after the other, and logs the messages it
/// returns. In a terminal, each is drawn as a run of that one step below `Docs: label`: queued, running, then
/// its messages.
pub(super) fn steps<M: Message + Serialize>(
    names: &[&str],
    label: &str,
    step: &str,
    mut f: impl FnMut(usize) -> Vec<M>,
) {
    if BOARD.runs.get().is_none() {
        for (i, name) in names.iter().enumerate() {
            for message in f(i) {
                log::event(Some(title(name)), &message);
            }
        }
        return;
    }
    let key = |name: &str| (name.to_owned(), OUTSIDE);
    BOARD.queued.lock().extend(names.iter().map(|name| (key(name), label.to_owned())));
    for (i, name) in names.iter().enumerate() {
        BOARD.queued.lock().remove(&key(name));
        *BOARD.doing.lock() =
            Some(Outside { collection: (*name).to_owned(), label: label.to_owned(), step: step.to_owned() });
        draw();
        let messages = f(i);
        let messages: Vec<&dyn Message> = messages.iter().map(|m| -> &dyn Message { m }).collect();
        // Together, so that no drawing in between shows neither.
        let mut doing = BOARD.doing.lock();
        *doing = None;
        BOARD.ended.lock().extend(section_lines(name, label, &messages));
        drop(doing);
        draw();
    }
}

/// Prints `messages` below `Docs: heading`.
fn section(collection: &str, heading: &str, messages: &[&dyn Message]) {
    log::runs(Vec::new(), &section_lines(collection, heading, messages));
}

/// `messages` below `Docs: heading`. A message's lines after its first go below it.
fn section_lines(collection: &str, heading: &str, messages: &[&dyn Message]) -> Vec<String> {
    let branches = messages.iter().map(|m| branch(m.level(), &m.to_string()));
    let head = format!("{}: {heading}", log::bold(title(collection)));
    iter::once(head).chain(tree(&branches.collect::<Vec<_>>()).lines().map(ToOwned::to_owned)).collect()
}

fn draw() {
    let Some(runs) = BOARD.runs.get().and_then(Weak::upgrade) else { return };
    let live: BTreeMap<_, _> = runs.all().into_iter().map(|l| ((l.collection.clone(), l.id), l)).collect();
    let doing = BOARD.doing.lock();
    let mut ended = std::mem::take(&mut *BOARD.ended.lock());
    let mut drawn = BOARD.drawn.lock();
    drawn.retain(|key, run| {
        let running = live.contains_key(key);
        if !running {
            ended.append(&mut run.lines);
            BOARD.shown.lock().append(&mut run.shown);
        }
        running
    });
    let mut queued = BOARD.queued.lock();
    queued.retain(|key, _| !live.contains_key(key));
    drawn.extend(live.into_iter().map(|(key, l)| (key, lines(&l))));
    let waiting = queued.iter().map(|((collection, id), label)| {
        let head = format!("{}: {label}", log::bold(title(collection)));
        let wait = runs.waiting(collection, *id).map(|w| format!(": {w:#}")).unwrap_or_default();
        let tree = tree(&[Tree::leaf(format!("{} {}{wait}", Tone::Muted.dot(), Status::Queued))]);
        ((collection.clone(), *id), iter::once(head).chain(tree.lines().map(ToOwned::to_owned)).collect())
    });
    let mut rows: BTreeMap<_, Vec<String>> = waiting.collect();
    rows.extend(drawn.iter().map(|(key, run)| (key.clone(), run.lines.clone())));
    if let Some(Outside { collection, label, step }) = doing.as_ref() {
        let head = format!("{}: {label}", log::bold(title(collection)));
        let tree = tree(&[Tree::leaf(format!("{} {step}", Tone::Running.dot()))]);
        rows.insert(
            (collection.clone(), OUTSIDE),
            iter::once(head).chain(tree.lines().map(ToOwned::to_owned)).collect(),
        );
    }
    log::runs(rows.into_values().flatten().collect(), &ended);
}

/// The run's label, then its steps with what each does now or did, then results no step produced.
fn lines(live: &Live) -> Drawn {
    let (record, activities) = (live.record(), live.activities());
    let steps = record.steps.iter().enumerate().map(|(i, step)| {
        let now = activities.iter().find(|a| a.step == Some(i) && step.status == Status::Running);
        let detail = match now {
            Some(a) => {
                a.progress.as_ref().map(Progress::summary).or_else(|| a.doing.as_ref().map(|d| format!("{d:#}")))
            }
            None => (!step.results.is_empty()).then(|| painted(step)),
        };
        let detail = detail.map(|d| format!(": {d}")).unwrap_or_default();
        Tree::leaf(format!("{} {}{detail}", step.status.tone().dot(), step.label))
    });
    let own = record.results.iter().map(|r| Tree::leaf(log::paint(r.level, &r.to_string())));
    let tree = tree(&steps.chain(own).collect::<Vec<_>>());
    let head = format!("{}: {}", log::bold(title(&record.collection)), record.label);
    let lines = iter::once(head).chain(tree.lines().map(ToOwned::to_owned)).collect();
    let shown = record.steps.iter().flat_map(|s| s.results.iter()).chain(&record.results).cloned().collect();
    Drawn { lines, shown }
}

/// Its results, painted.
fn painted(step: &StepRecord) -> String {
    step.results.iter().map(|r| log::paint(r.level, &step.result(r))).collect::<Vec<_>>().join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::tree::below;
    use crate::log::Level;

    struct Said(Level, &'static str);

    impl std::fmt::Display for Said {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.1)
        }
    }

    impl Message for Said {
        fn level(&self) -> Level {
            self.0
        }
    }

    #[test]
    fn messages_are_drawn_below_their_collection() {
        let backed_up = Said(Level::Success, "backed up");
        let notified = Said(Level::Failure, "not notified\nDocs is failing\n🔴 missing source\n🟡 1 file skipped");
        let lines = section_lines("Docs", "backup", &[&backed_up, &notified]);
        assert_eq!(
            lines,
            [
                "Docs: backup",
                "├── ● backed up",
                "└──┐● not notified",
                "   └──┐Docs is failing",
                "      ├── ● missing source",
                "      └── ● 1 file skipped",
            ]
        );
    }

    #[test]
    fn an_issue_without_a_line_before_it_stands_alone() {
        let drawn = tree(&below("🟡 1 file skipped\nafter".lines()));
        assert_eq!(drawn, "├── ● 1 file skipped\n└── after\n");
        assert_eq!(section_lines("Docs", "check", &[]), ["Docs: check"]);
    }

    /// A running step shows what it does now, a step that ended its results, and a step without either
    /// its label only.
    #[test]
    fn a_run_shows_what_each_step_does_or_did() {
        use crate::{
            live::{Guard, Runs},
            request::{Request, Trigger},
            result::{Doing, StepResult},
            state::{RunLog, StepRecord, Store},
        };
        let store = Store::at(crate::sys::test_dir("board-lines")).unwrap();
        let run = Guard::start(&Runs::new(|_| {}), RunLog::new(&store, "Docs", Request::Backup, Trigger::Cli));
        let step = |label: &str, repo: Option<&str>, status, results: &[StepResult]| StepRecord {
            repo: repo.map(Into::into),
            status,
            results: results.iter().map(Into::into).collect(),
            ..StepRecord::queued(label.into(), Vec::new())
        };
        let copied = StepResult::Copied { target: "Docs-usb".into(), snapshots: 2 };
        run.live.steps(&[
            step("backup", None, Status::Running, &[]),
            step("copy to Docs-usb", Some("Docs-usb"), Status::Succeeded, &[copied]),
            step("forget", None, Status::Running, &[]),
            step("prune", None, Status::Queued, &[]),
        ]);
        run.live.set_activity(Some(0), Some(Doing::Path { path: "/docs".into() }), None);
        // Not running: what it did shows instead.
        run.live.set_activity(Some(1), Some(Doing::Path { path: "/old".into() }), None);
        let drawn = lines(&run.live);
        insta::assert_debug_snapshot!(drawn.lines);
        // What the run showed is left out of its report.
        let mut report = RunResults::new("Docs");
        report.push(StepResult::Copied { target: "Docs-usb".into(), snapshots: 2 });
        report.push(StepResult::InSync { target: "Docs-usb".into() });
        BOARD.shown.lock().extend(drawn.shown);
        leave_out(&mut report);
        assert_eq!(report.results, [StepResult::InSync { target: "Docs-usb".into() }]);
    }
}
