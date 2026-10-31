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
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(dir.join(format!("tests/fixtures/rustic-{}.{}/{name}", TESTED.major, TESTED.minor)))
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

#[test]
fn refresh_measures_how_far_a_target_is_behind() {
    let replay = Replay::new()
        .ok("Docs", &["snapshots", "--json"], &recorded("snapshots.stdout"))
        .ok("Docs", &["repoinfo"], &recorded("repoinfo.stdout"))
        .ok("Docs-usb", &["snapshots", "--json"], &recorded("snapshots-copied.stdout"))
        .ok("Docs-usb", &["repoinfo"], &recorded("repoinfo.stdout"));
    let engine = engine("refresh", &replay);
    let collections = engine.collections();
    let report = engine.refresh(&collections[0]);
    let mut outcome = outcome(&replay, &report, &engine, &collections[0]);
    // Listed in parallel, in no particular order.
    outcome.rustic.sort();
    insta::assert_json_snapshot!(outcome);
}

#[test]
fn a_backup_is_copied_then_forgotten_where_every_target_has_it() {
    let snapshots = recorded("snapshots.stdout");
    let replay = Replay::new()
        // Each repository's id, read once for its locks.
        .ok("Docs", &["cat", "config"], &recorded("cat-config.stdout"))
        .ok("Docs-usb", &["cat", "config"], &recorded("cat-config.stdout"))
        // Exists already, so it is not created.
        .ok("Docs", &["snapshots", "--json"], &snapshots)
        .ok("Docs", &["backup", "--json"], &recorded("backup.stdout"))
        .ok("Docs", &["cat", "snapshot"], &recorded("cat-snapshot.stdout"))
        .ok("Docs", &["snapshots", "--json"], &snapshots)
        .ok("Docs", &["repoinfo"], &recorded("repoinfo.stdout"))
        .ok("Docs-usb", &["snapshots", "--json"], &recorded("snapshots-copied.stdout"))
        .ok("Docs", &["copy"], "")
        .ok("Docs-usb", &["snapshots", "--json"], &recorded("snapshots-copied-both.stdout"))
        .ok("Docs-usb", &["repoinfo"], &recorded("repoinfo.stdout"))
        .ok("Docs-usb", &["forget", "--json"], &recorded("forget.stdout"))
        .ok("Docs", &["forget", "--json"], &recorded("forget.stdout"));
    let engine = engine("backup", &replay);
    let collections = engine.collections();
    let report = asked(&engine, &collections[0], &Request::Backup);
    insta::assert_json_snapshot!(forgets_sorted(outcome(&replay, &report, &engine, &collections[0])));
}

/// Runs the request as a command would.
fn asked(engine: &Engine, collection: &Collection<'_>, request: &Request) -> RunResults {
    let Ok(Asked::Queued(queued)) = engine.ask(collection, request, Trigger::Cli) else { panic!("not queued") };
    engine.execute(collection, queued, &Batch::new(Trigger::Cli))
}

/// Rustic exits 0 when a copy target fails. Listed afterwards, and again because a remote's listing may
/// lag, the target still lacks the snapshot: the copy failed, its forget is skipped, and the
/// primary's forget waits.
#[test]
fn an_unconfirmed_copy_holds_back_the_forgets() {
    let lacking = recorded("snapshots-copied.stdout");
    let replay = Replay::new()
        // Each repository's id, read once for its locks.
        .ok("Docs", &["cat", "config"], &recorded("cat-config.stdout"))
        .ok("Docs-usb", &["cat", "config"], &recorded("cat-config.stdout"))
        .ok("Docs", &["snapshots", "--json"], &recorded("snapshots.stdout"))
        .ok("Docs-usb", &["snapshots", "--json"], &lacking)
        .ok("Docs", &["copy"], "")
        .ok("Docs-usb", &["snapshots", "--json"], &lacking)
        .ok("Docs-usb", &["snapshots", "--json"], &lacking)
        .ok("Docs-usb", &["snapshots", "--json"], &lacking)
        .ok("Docs-usb", &["snapshots", "--json"], &lacking);
    let engine = engine("unconfirmed-copy", &replay);
    let collections = engine.collections();
    let report = asked(&engine, &collections[0], &Request::Copy("Docs".into()));
    insta::assert_json_snapshot!(outcome(&replay, &report, &engine, &collections[0]));
}

/// A remote lists a copied snapshot only after a while. The copy succeeds once it does.
#[test]
fn a_lagging_listing_is_waited_for() {
    let replay = Replay::new()
        // Each repository's id, read once for its locks.
        .ok("Docs", &["cat", "config"], &recorded("cat-config.stdout"))
        .ok("Docs-usb", &["cat", "config"], &recorded("cat-config.stdout"))
        .ok("Docs", &["snapshots", "--json"], &recorded("snapshots.stdout"))
        .ok("Docs-usb", &["snapshots", "--json"], &recorded("snapshots-copied.stdout"))
        .ok("Docs", &["copy"], "")
        .ok("Docs-usb", &["snapshots", "--json"], &recorded("snapshots-copied.stdout"))
        .ok("Docs-usb", &["snapshots", "--json"], &recorded("snapshots-copied-both.stdout"))
        .ok("Docs-usb", &["repoinfo"], &recorded("repoinfo.stdout"))
        .ok("Docs-usb", &["forget", "--json"], &recorded("forget.stdout"))
        .ok("Docs", &["forget", "--json"], &recorded("forget.stdout"));
    let engine = engine("lagging-listing", &replay);
    let collections = engine.collections();
    let report = asked(&engine, &collections[0], &Request::Copy("Docs".into()));
    insta::assert_json_snapshot!(forgets_sorted(outcome(&replay, &report, &engine, &collections[0])));
}

/// A notification that cannot be sent is attempted again once its backoff has passed, not before.
#[test]
fn a_failed_notification_is_sent_again_after_its_backoff() {
    use crate::{request::StepKey, result::StepResult, state::Event};
    let docs = format!("{DOCS}[global.env]\n{} = \"json://127.0.0.1:1\"\n", crate::settings::NOTIFY.key);
    let engine = engine_with("notify-backoff", &Replay::new(), &docs);
    let at = engine.store.now();
    let result = StepResult::Error { error: "repository unreachable".into() };
    let failed =
        Event::Attempted { key: StepKey::Snapshot("Docs".into()), started: at, at, result: (&result).into(), by: None };
    engine.store.apply(&[failed]).unwrap();
    let collection = &engine.collections()[0];
    let retry = engine.notify(collection).unwrap();
    assert_eq!(engine.store.notified("Docs").failures, 1);
    assert_eq!(engine.notify(collection), Some(retry), "not before its backoff");
    assert_eq!(engine.store.notified("Docs").failures, 1);
    engine.store.wait(retry.duration_since(at).unsigned_abs(), &|| false);
    assert!(engine.notify(collection) > Some(retry), "sent again, and failed again");
    assert_eq!(engine.store.notified("Docs").failures, 2);
}

/// A scheduled step's first failure shows its last success until the failure counts.
/// Without profiles nothing is backed up: a warning on the first start, a failure once collections had run.
#[test]
fn no_profile_is_a_warning_at_first_and_a_failure_after_runs() {
    use crate::{log::Message, request::StepKey, result::StepResult, state::Event};
    let engine = engine_of("unconfigured", &Replay::new(), &[]);
    assert_eq!(engine.unconfigured(), Some(super::status::Unconfigured::New));
    assert_eq!(engine.report().level, Level::Warning);
    let at = engine.store.now();
    let ran = Event::Attempted {
        key: StepKey::Snapshot("Docs".into()),
        started: at,
        at,
        result: (&StepResult::Error { error: "gone".into() }).into(),
        by: None,
    };
    engine.store.apply(&[ran]).unwrap();
    assert_eq!(engine.unconfigured().map(|u| u.level()), Some(Level::Failure));
    assert_eq!(engine.report().level, Level::Failure);
}

#[test]
fn a_pending_failure_shows_the_last_success() {
    use crate::{
        request::StepKey,
        result::StepResult,
        state::{Event, RunStep},
    };
    let engine = engine("pending-failure", &Replay::new());
    let at = engine.store.now();
    let by = Some(RunStep { collection: "Docs".into(), run: 1, step: None, scheduled: true });
    let result = StepResult::Error { error: "repository unreachable".into() };
    engine
        .store
        .apply(&[Event::Attempted {
            key: StepKey::Check("Docs".into()),
            started: at,
            at,
            result: (&result).into(),
            by,
        }])
        .unwrap();
    let checked = || engine.status_of(&engine.collections()[0]).primary.checked;
    assert!(checked() == super::status::Last::Never);
    engine.store.wait(std::time::Duration::from_mins(6), &|| false);
    assert!(checked() == super::status::Last::Failing(1));
}

/// Of the groups rustic makes, one per source belongs to the collection: this host's, else the newest one.
#[test]
fn a_job_keeps_its_own_hosts_snapshots() {
    let collection = |extra: &str| profile::parse("Docs", &format!("{DOCS}[backup]\nhost = \"laptop\"\n{extra}"));
    let group = |host: Option<&str>, snapshots: &[(&str, &str)]| -> crate::rustic::SnapshotGroup {
        let snapshots: Vec<_> = snapshots
            .iter()
            .map(|(id, time)| serde_json::json!({"id": format!("{id:0<64}"), "time": time, "paths": ["/fixture/src/docs"], "hostname": host.unwrap_or("desktop")}))
            .collect();
        serde_json::from_value(serde_json::json!({"group_key": {"hostname": host}, "snapshots": snapshots})).unwrap()
    };
    let ids =
        |snapshots: Vec<crate::rustic::Snapshot>| snapshots.iter().map(|s| s.id.short().to_owned()).collect::<Vec<_>>();
    let laptop = || group(Some("laptop"), &[("a", "2026-10-01T00:00:00Z")]);
    let desktop = || group(Some("desktop"), &[("b", "2026-10-02T00:00:00Z")]);
    // Another computer's newer snapshot of the same paths is not this host's.
    assert_eq!(ids(super::snapshots::own(&collection(""), &[laptop(), desktop()])), ["a0000000"]);
    // A source's own `host` selects its group, even if older.
    let own_host = profile::parse("Docs", &DOCS.replace("as-path", "host = \"laptop\"\nas-path"));
    assert_eq!(ids(super::snapshots::own(&own_host, &[laptop(), desktop()])), ["a0000000"]);
    // Reinstalled with another host name: the newest group, to restore from.
    assert_eq!(ids(super::snapshots::own(&profile::parse("Docs", DOCS), &[laptop(), desktop()])), ["b0000000"]);
    // A `group-by` without `host`: every host's snapshots, as configured.
    let together = group(None, &[("a", "2026-10-01T00:00:00Z"), ("b", "2026-10-02T00:00:00Z")]);
    assert_eq!(
        ids(super::snapshots::own(&collection("[global]\ngroup-by = \"paths\"\n"), &[together])),
        ["a0000000", "b0000000"]
    );
}

/// A source that is nearly empty again keeps its older, larger snapshots from forget. A small source
/// does not.
#[test]
fn a_source_that_shrank_holds_its_older_snapshots() {
    let snapshot = |id: &str, time: &str, bytes: u64| -> crate::rustic::Snapshot {
        serde_json::from_value(serde_json::json!({
            "id": format!("{id:0<64}"), "time": time, "paths": ["/docs"],
            "summary": {"total_bytes_processed": bytes}
        }))
        .unwrap()
    };
    let full = || snapshot("a", "2026-10-01T00:00:00Z", 10 << 20);
    let held = |snapshots: &[crate::rustic::Snapshot]| -> Vec<String> {
        super::shrunk(snapshots).into_iter().flat_map(|(_, held)| held).map(|s| s.id.short().to_owned()).collect()
    };
    assert_eq!(held(&[full(), snapshot("b", "2026-10-02T00:00:00Z", 1024)]), ["a0000000"]);
    // Half the size: the sources changed, nothing is held.
    assert!(held(&[full(), snapshot("b", "2026-10-02T00:00:00Z", 5 << 20)]).is_empty());
    // Too small to tell.
    assert!(held(&[snapshot("a", "2026-10-01T00:00:00Z", 1000), snapshot("b", "2026-10-02T00:00:00Z", 0)]).is_empty());
}

/// A recovery says how long the level notified before it lasted, from when that level was delivered, and
/// how many other collections notified at the same URLs are not healthy.
#[test]
fn a_recovery_notification_says_how_long_the_previous_level_lasted() {
    use crate::{request::StepKey, result::StepResult, state::Event, state::Notified};
    let elsewhere = "[repository]\nrepository = \"rest:http://backup/music\"\n[[backup.snapshots]]\nsources = [\"/music\"]\n[global.env]\nRUSTICLONE_NOTIFY_URLS = \"ntfy://ntfy.sh/music\"\n";
    let photos =
        "[repository]\nrepository = \"rest:http://backup/photos\"\n[[backup.snapshots]]\nsources = [\"/photos\"]\n";
    let profiles = [("Docs", DOCS), ("Docs-usb", USB), ("Music", elsewhere), ("Photos", photos)];
    let engine = engine_of("notify-recovery", &Replay::new(), &profiles);
    let (key, at) = (StepKey::Snapshot("Docs".into()), engine.store.now());
    let result = StepResult::Error { error: "repository unreachable".into() };
    let failed = |name: &str| Event::Attempted {
        key: StepKey::Snapshot(name.into()),
        started: at,
        at,
        result: (&result).into(),
        by: None,
    };
    engine.store.apply(&["Docs", "Music", "Photos"].map(failed)).unwrap();
    let failing = Notified { level: Level::Failure, since: Some(at), ..Notified::default() };
    engine.store.update_notified("Docs", |_| failing.clone()).unwrap();
    let collection = &engine.collections()[0];
    let host = crate::sys::hostname();
    let text = engine.message(collection, &engine.status_of(collection), &failing);
    assert_eq!(text, format!("{host}: Docs is now failing\n🔴 Repository unreachable\n"), "no age unless recovered");
    engine.store.wait(std::time::Duration::from_hours(51), &|| false);
    let (at, result) =
        (engine.store.now(), StepResult::BackedUp { repo: "Docs".into(), snapshots: vec![], new_repository: false });
    engine.store.apply(&[Event::Attempted { key, started: at, at, result: (&result).into(), by: None }]).unwrap();
    let text = engine.message(collection, &engine.status_of(collection), &failing);
    let others = "🔴 1 other collection failing";
    assert_eq!(text, format!("{host}: Docs is now healthy\n🟢 Was failing for 2 days\n{others}\n"));
}

/// Recorded once in `tests/fixtures/status.json`: what `/status` answers may gain keys, but every recorded
/// one keeps its type, so that other programs keep reading it. Deleting the file records it again.
#[test]
fn status_keeps_its_shape() {
    use crate::{
        request::StepKey,
        result::StepResult,
        state::{Behind, Event},
    };
    use serde_json::Value;
    /// Every key of `old` is in `new`, with a value of the same type. `null` is any type.
    fn keeps(old: &Value, new: &Value) -> bool {
        match (old, new) {
            (Value::Null, _) | (_, Value::Null) => true,
            (Value::Object(o), Value::Object(n)) => o.iter().all(|(k, v)| n.get(k).is_some_and(|n| keeps(v, n))),
            (Value::Array(o), Value::Array(n)) => o.iter().zip(n).all(|(o, n)| keeps(o, n)),
            (o, n) => std::mem::discriminant(o) == std::mem::discriminant(n),
        }
    }
    let env = [crate::settings::BACKUP, crate::settings::CHECK, crate::settings::PRUNE];
    let env: String = env.iter().map(|s| format!("{} = \"@daily\"\n", s.key)).collect();
    let engine = engine_with("status-shape", &Replay::new(), &format!("{DOCS}[global.env]\n{env}"));
    let at = engine.store.now();
    let (backup, copy) = (StepKey::Snapshot("Docs".into()), StepKey::Copy("Docs".into(), "Docs-usb".into()));
    let backed_up =
        StepResult::BackedUp { repo: "Docs".into(), snapshots: vec!["22222222".into()], new_repository: false };
    let skipped = StepResult::Skipped { items: vec![("/a".into(), "denied".into())], count: 1 };
    let failed = StepResult::Failed { step: "copy to Docs-usb".into(), error: "unreachable".into(), stopped: false };
    engine
        .store
        .apply(&[
            Event::Attempted { key: backup.clone(), started: at, at, result: (&backed_up).into(), by: None },
            Event::Warned { key: backup, warnings: vec![(&skipped).into()] },
            Event::Attempted { key: copy, started: at, at, result: (&failed).into(), by: None },
            Event::Lag { collection: "Docs".into(), target: "Docs-usb".into(), at, behind: Behind::Listed(Some(at)) },
        ])
        .unwrap();
    let report = crate::shape::to_value(&engine.report()).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/status.json");
    if !path.exists() {
        std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    }
    let stored: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(keeps(&stored, &report), "{report:#}\nlacks part of the recorded shape:\n{stored:#}");
}

#[test]
fn a_repository_narrows_the_collection_named_with_it() {
    use crate::request::Selection;
    let pics = DOCS.replace("docs\"", "pics\"");
    let rustic_profiles: BTreeMap<_, _> = [("Docs", DOCS), ("Docs-usb", USB), ("Pics", pics.as_str())]
        .map(|(name, config)| (name.into(), profile::parse(name, config)))
        .into();
    let profiles = rustic_profiles.keys().cloned().collect();
    let store = Store::at(crate::sys::test_dir("engine-narrows")).unwrap();
    let mut engine = Engine::with_profiles(
        Rustic::with_runner(Arc::new(Replay::default())),
        store,
        Runs::new(|_| {}),
        rustic_profiles,
        Vec::new(),
        profiles,
        Named::default(),
    );
    let mut selections = |collections: &[&str]| {
        let collections = collections.iter().map(|&c| c.into()).collect();
        engine.named = Named { collections, repos: vec!["Docs-usb".into()] };
        engine.selections().unwrap()
    };
    let usb = |c: &str| Selection::Repo(c.into(), "Docs-usb".into());
    assert_eq!(selections(&[]), [usb("Docs"), usb("Pics")]);
    assert_eq!(selections(&["Docs"]), [usb("Docs")]);
}

/// Metrics name an issue's kind as `/status` does.
#[cfg(feature = "server")]
#[test]
fn issue_kinds_match_their_json() {
    let issues = [
        super::Issue::NeverBackedUp,
        super::Issue::Missing { repo: "Docs".into(), primary: true },
        super::Issue::Unreadable { error: String::new() },
    ];
    for issue in issues {
        assert_eq!(serde_json::to_value(&issue).unwrap()["kind"], <&str>::from(&issue));
    }
}

/// A request joins the same one queued, but not a copy to one target the copy to all, nor a check
/// another, which reads the next data subset.
#[test]
fn a_request_joins_the_same_one_queued() {
    let engine = engine("join", &Replay::new());
    let collections = engine.collections();
    let ask = |request: Request| engine.ask(&collections[0], &request, Trigger::Cli).unwrap();
    let first = ask(Request::Backup);
    assert!(matches!(ask(Request::Backup), Asked::Joined(r) if r.id == first.record().id));
    let to_usb = ask(Request::Copy("Docs-usb".into()));
    assert!(matches!(ask(Request::Copy("Docs".into())), Asked::Queued(_)));
    assert!(matches!(ask(Request::Copy("Docs-usb".into())), Asked::Joined(r) if r.id == to_usb.record().id));
    let checks = [ask(Request::Check(vec![("Docs".into(), None)])), ask(Request::Check(vec![("Docs".into(), None)]))];
    assert!(checks.iter().all(|c| matches!(c, Asked::Queued(_))));
}

/// A scheduled run shows what it waits for as it does a lock, each in turn, then goes on. A run started by hand never
/// waits.
#[cfg(feature = "server")]
#[test]
fn a_scheduled_run_waits_for_power_then_an_unmetered_network() {
    use super::waiting::{Waiting, wait_while};
    use crate::{live::Guard, request::Trigger, state::RunLog};
    let engine = engine("waiting", &Replay::new());
    let collections = engine.collections();
    let start = |trigger| {
        let log = RunLog::new(&engine.store, collections[0].name(), Request::Backup, trigger);
        Guard::start(&engine.runs, log)
    };
    let scheduled = start(Trigger::Schedule);
    let run = super::Run::new(&collections[0], Some(scheduled.live.clone()));
    let (mut answers, mut shown) =
        (vec![None, Some(Waiting::UnmeteredNetwork), Some(Waiting::PowerSupply)], Vec::new());
    let waited = wait_while(&run, std::time::Duration::ZERO, || {
        shown.push(scheduled.live.activities().into_iter().find_map(|a| a.doing).map(|d| d.to_string()));
        answers.pop().flatten()
    });
    let doing = |w: Waiting| {
        Some(crate::result::Doing::WaitingFor { lock: w.resource().to_owned(), by: Vec::new() }.to_string())
    };
    assert!(waited);
    assert_eq!(shown, [None, doing(Waiting::PowerSupply), doing(Waiting::UnmeteredNetwork)]);
    let by_hand = start(Trigger::Web);
    let run = super::Run::new(&collections[0], Some(by_hand.live.clone()));
    assert!(!wait_while(&run, std::time::Duration::ZERO, || Some(Waiting::PowerSupply)));
}
