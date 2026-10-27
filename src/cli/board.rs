//! In a terminal, each run of this process as its plan of steps, redrawn as it goes. A run that ends
//! stays printed, and its results are left out of the report that follows.

use std::{
    collections::BTreeMap,
    iter,
    sync::{Arc, OnceLock, Weak},
    time::Duration,
};

use parking_lot::Mutex;

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

/// A run as last drawn.
struct Drawn {
    lines: Vec<String>,
    /// The results the lines show.
    shown: Vec<ResultRecord>,
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
}

static BOARD: Board = Board {
    runs: OnceLock::new(),
    titles: OnceLock::new(),
    drawn: Mutex::new(BTreeMap::new()),
    queued: Mutex::new(BTreeMap::new()),
    shown: Mutex::new(Vec::new()),
};

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
    let results = report
        .results
        .iter()
        .map(|r| Tree::leaf(format!("{} {}", Tone::from(r.level()).dot(), log::paint(r.level(), &r.to_string()))));
    let head = format!("{}: {heading}", log::bold(title(&report.collection)));
    let lines: Vec<String> =
        iter::once(head).chain(tree(&results.collect::<Vec<_>>()).lines().map(ToOwned::to_owned)).collect();
    log::runs(Vec::new(), &lines);
}

fn draw() {
    let Some(runs) = BOARD.runs.get().and_then(Weak::upgrade) else { return };
    let live: BTreeMap<_, _> = runs.all().into_iter().map(|l| ((l.collection.clone(), l.id), l)).collect();
    let mut drawn = BOARD.drawn.lock();
    let mut ended = Vec::new();
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
