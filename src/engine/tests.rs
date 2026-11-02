//! The engine with recorded rustic output (`tests/fixtures`) and a fixed clock: what it asks rustic,
//! what it reports, and the collection's status afterwards.

#![expect(clippy::disallowed_methods, reason = "tests read fixtures and use a state on disk")]

use std::{collections::BTreeMap, sync::Arc};

use jiff::tz::TimeZone;

use serde::Serialize;

use super::{Asked, Batch, Collection, CollectionOverview, Engine, Named};
use crate::request::Request;
use crate::{
    clock::Fixed,
    live::Runs,
    log::{Level, Message},
    profile,
    request::Trigger,
    result::RunResults,
    rustic::{Replay, Rustic, TESTED},
    state::{StepRecord, Store},
};

mod backup;
mod copy;
mod maintain;
mod notify;
mod queue;
mod schedule;
mod status;

/// The collection `Docs`, backed up to a REST server and copied to `Docs-usb`, with one snapshot group, as
/// in the recorded fixtures.
const DOCS: &str = r#"
[repository]
repository = "rest:http://backup/docs"
password = "test"
[[backup.snapshots]]
sources = ["-"]
as-path = "/fixture/src/docs"
[copy]
targets = ["Docs-usb"]
[forget]
keep-last = 1
"#;

const USB: &str = r#"
[repository]
repository = "rest:http://usb/docs"
password = "test"
[forget]
keep-last = 1
"#;

fn recorded(name: &str) -> String {
    let dir = crate::tests::root();
    std::fs::read_to_string(
        dir.join("tests").join("fixtures").join(format!("rustic-{}.{}", TESTED.major, TESTED.minor)).join(name),
    )
    .unwrap()
}

fn engine(test: &str, replay: &Arc<Replay>) -> Engine {
    engine_with(test, replay, DOCS)
}

/// With `docs` as the collection's profile.
fn engine_with(test: &str, replay: &Arc<Replay>, docs: &str) -> Engine {
    engine_of(test, replay, &[("Docs", docs), ("Docs-usb", USB)])
}

/// With these profiles, by name.
fn engine_of(test: &str, replay: &Arc<Replay>, profiles: &[(&str, &str)]) -> Engine {
    let store = Store::at(crate::sys::test_dir(&format!("engine-{test}")))
        .unwrap()
        .with_clock(Fixed::at("2026-10-16T12:00:00Z"));
    let rustic_profiles: BTreeMap<_, _> =
        profiles.iter().map(|&(name, config)| (name.into(), profile::parse(name, config))).collect();
    let rustic = Rustic::with_runner(replay.clone());
    let profiles = rustic_profiles.keys().cloned().collect();
    let mut engine = Engine::with_profiles(
        rustic,
        store,
        Runs::new(|_| {}),
        rustic_profiles,
        Vec::new(),
        profiles,
        Named::default(),
    );
    engine.tz = TimeZone::UTC;
    engine
}

/// What rustic was asked, what the run reported, and the collection's status afterwards.
#[derive(Serialize)]
struct Outcome {
    rustic: Vec<String>,
    report: Vec<(Level, String)>,
    /// Empty if no run was made.
    steps: Vec<StepRecord>,
    status: CollectionOverview,
}

/// The forgets that end a run in sorted order. Each repository forgets once it has the copies and in parallel
/// with the others: they end in no particular order.
fn forgets_sorted(mut outcome: Outcome) -> Outcome {
    let start =
        |lines: &[String], word: &str| lines.len() - lines.iter().rev().take_while(|l| l.contains(word)).count();
    let at = start(&outcome.rustic, " forget ");
    outcome.rustic[at..].sort();
    let lines: Vec<String> = outcome.report.iter().map(|(_, l)| l.clone()).collect();
    let at = start(&lines, ": forgot ");
    outcome.report[at..].sort_by(|a, b| a.1.cmp(&b.1));
    outcome
}

/// Asserts that every answer was used.
fn outcome(replay: &Replay, report: &RunResults, engine: &Engine, collection: &Collection<'_>) -> Outcome {
    assert!(replay.unused().is_empty(), "never asked: {:?}", replay.unused());
    let steps = engine.store.runs(collection.name(), 1).into_iter().flat_map(|r| r.steps).collect();
    Outcome {
        rustic: replay.calls(),
        report: report.results.iter().map(|l| (l.level(), l.to_string())).collect(),
        steps,
        status: engine.status_of(collection),
    }
}

/// Runs the request as a command would.
fn asked(engine: &Engine, collection: &Collection<'_>, request: &Request) -> RunResults {
    let Ok(Asked::Queued(queued)) = engine.ask(collection, request, Trigger::Cli) else { panic!("not queued") };
    engine.execute(collection, queued, &Batch::new(Trigger::Cli))
}

/// The profile of `DOCS` with these settings, as `[global.env]`.
fn docs_with(settings: &[(&crate::settings::Setting, &str)]) -> String {
    let env: String = settings.iter().map(|(s, v)| format!("{} = \"{v}\"\n", s.key)).collect();
    format!("{DOCS}[global.env]\n{env}")
}

/// `key` attempted now with `result`, by a scheduled run if `scheduled`.
fn attempted(engine: &Engine, key: crate::request::StepKey, result: &crate::result::StepResult, scheduled: bool) {
    use crate::state::{Event, RunStep};
    let (at, by) =
        (engine.store.now(), scheduled.then(|| RunStep { collection: "Docs".into(), run: 1, step: None, scheduled }));
    engine.store.apply(&[Event::Attempted { key, started: at, at, result: result.into(), by }]).unwrap();
}

fn backed_up() -> crate::result::StepResult {
    crate::result::StepResult::BackedUp { repo: "Docs".into(), snapshots: vec![], new_repository: false }
}

fn failed() -> crate::result::StepResult {
    crate::result::StepResult::Error { error: "repository unreachable".into() }
}

fn at(time: &str) -> jiff::Timestamp {
    time.parse().unwrap()
}
