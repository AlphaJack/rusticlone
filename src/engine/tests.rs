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
    assert_eq!(engine.unconfigured(), Some(super::report::Unconfigured::New));
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
    assert!(checked() == super::report::Last::Never);
    engine.store.wait(std::time::Duration::from_mins(6), &|| false);
    assert!(checked() == super::report::Last::Failing(1));
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
    let pics = photos.replace("photos", "pics");
    let profiles = [("Docs", DOCS), ("Docs-usb", USB), ("Music", elsewhere), ("Photos", photos), ("Pics", &pics)];
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
    engine.store.apply(&["Docs", "Music", "Photos", "Pics"].map(failed)).unwrap();
    let failing = Notified { level: Level::Failure, since: Some(at), ..Notified::default() };
    engine.store.update_notified("Docs", |_| failing.clone()).unwrap();
    let collection = &engine.collections()[0];
    let host = crate::sys::hostname().unwrap();
    let text = engine.message(collection, &engine.status_of(collection), &failing);
    assert_eq!(text, format!("{host}: Docs is now failing\n🔴 Repository unreachable\n"), "no age unless recovered");
    engine.store.wait(std::time::Duration::from_hours(51), &|| false);
    let (at, result) =
        (engine.store.now(), StepResult::BackedUp { repo: "Docs".into(), snapshots: vec![], new_repository: false });
    engine.store.apply(&[Event::Attempted { key, started: at, at, result: (&result).into(), by: None }]).unwrap();
    let text = engine.message(collection, &engine.status_of(collection), &failing);
    // Those notified at the same URLs: not Music.
    let others = "🔴 2 other collections failing";
    assert_eq!(text, format!("{host}: Docs is now healthy\n🟢 Was failing for 2 days\n{others}\n"));
    // Healthy before too: nothing lasted.
    let healthy = Notified { level: Level::Success, since: Some(at), ..Notified::default() };
    assert_eq!(
        engine.message(collection, &engine.status_of(collection), &healthy),
        format!("{host}: Docs is now healthy\n")
    );
}

/// A level already notified is not sent again. Sends at the same level keep when it started.
#[test]
fn a_level_is_notified_once_from_when_it_started() {
    use std::io::{Read, Write};

    use crate::state::Notified;
    let receiver = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("json://{}", receiver.local_addr().unwrap());
    let answering = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut socket, _) = receiver.accept().unwrap();
            let mut request = [0; 4096];
            _ = socket.read(&mut request).unwrap();
            write!(socket, "HTTP/1.1 204 Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        }
    });
    let docs = format!("{DOCS}[global.env]\n{} = \"{url}\"\n", crate::settings::NOTIFY.key);
    let engine = engine_with("notify-once", &Replay::new(), &docs);
    let collection = &engine.collections()[0];
    // Notified at this level, with a failure left from an earlier one: nothing is sent, which would fail.
    let refused = format!("{DOCS}[global.env]\n{} = \"json://127.0.0.1:1\"\n", crate::settings::NOTIFY.key);
    let unsent = engine_with("notify-unsent", &Replay::new(), &refused);
    let level = unsent.status_of(&unsent.collections()[0]).level;
    let left = Notified { level, failures: 2, error: Some("refused".into()), ..Notified::default() };
    unsent.store.update_notified("Docs", |_| left.clone()).unwrap();
    unsent.notify(&unsent.collections()[0]);
    let notified = unsent.store.notified("Docs");
    assert_eq!((notified.level, notified.failures, notified.error), (level, 0, None));
    engine.notify_now(collection, &mut |_| {}).unwrap();
    let first = engine.store.notified("Docs").since;
    engine.store.wait(std::time::Duration::from_hours(1), &|| false);
    engine.notify_now(collection, &mut |_| {}).unwrap();
    assert_eq!(engine.store.notified("Docs").since, first);
    answering.join().unwrap();
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
    let path = crate::tests::root().join("tests").join("fixtures").join("status.json");
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
    assert_eq!(selections(&["Docs", "Pics"]), [usb("Docs"), usb("Pics")]);
    // A repository of another collection leaves the named one whole.
    engine.named = Named { collections: vec!["Docs".into()], repos: vec!["Pics".into()] };
    let both = [Selection::Collection("Docs".into()), Selection::Repo("Pics".into(), "Pics".into())];
    assert_eq!(engine.selections().unwrap(), both);
    // Only the named collections are loaded.
    engine.named = Named { collections: vec!["Pics".into()], repos: Vec::new() };
    let names: Vec<String> = engine.collections().iter().map(|c| c.name().to_owned()).collect();
    assert_eq!(names, ["Pics"]);
}

/// A copy of every collection leaves out those that cannot copy, which a named one fails on. A copy target is
/// unusable as broken only if its profile is.
#[test]
fn a_copy_of_every_collection_leaves_out_those_without_targets() {
    use crate::request::{RestoreOptions, RunKind, Selection};
    // A collection of its own, without copy targets.
    let lone = DOCS
        .replace("backup/docs", "backup/lone")
        .replace("src/docs", "src/lone")
        .replace("[copy]\ntargets = [\"Docs-usb\"]\n", "");
    assert!(!lone.contains("[copy]"));
    let mut engine = engine_of("copy-all", &Replay::new(), &[("Docs", DOCS), ("Docs-usb", USB), ("Lone", &lone)]);
    let none = RestoreOptions::default();
    let copies = engine.requests(&Selection::All, RunKind::Copy, &none).unwrap();
    let names: Vec<&str> = copies.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["Docs"]);
    assert!(engine.requests(&Selection::Collection("Lone".into()), RunKind::Copy, &none).is_err());
    engine.broken = vec![("Docs-usb".into(), "unreadable".into())];
    assert!(engine.unusable("Docs-usb").broken && !engine.unusable("Docs-nas").broken);
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

/// A backup never made is due now, so nothing is due next. After one, the schedule's next time is.
#[test]
fn the_next_run_is_after_now() {
    use crate::request::StepKey;
    let engine = engine_with("next-due", &Replay::new(), &docs_with(&[(&crate::settings::BACKUP, "@daily")]));
    let collection = &engine.collections()[0];
    assert_eq!(engine.next_due(collection), None, "due now");
    attempted(&engine, StepKey::Snapshot("Docs".into()), &backed_up(), false);
    assert_eq!(engine.next_due(collection), Some((Request::Backup, at("2026-10-17T00:00:00Z"))));
}

/// A failed copy is due again at its retry, not at once.
#[test]
fn a_failed_copy_waits_for_its_retry() {
    use crate::request::StepKey;
    let engine = engine("copy-retry", &Replay::new());
    let collection = &engine.collections()[0];
    let key = StepKey::Copy("Docs".into(), "Docs-usb".into());
    attempted(&engine, key.clone(), &failed(), false);
    let retry = engine.store.load().last_step(&key).unwrap().retry_at().unwrap();
    assert!(retry > engine.store.now());
    assert_eq!(engine.next_due(collection), Some((Request::Copy("Docs".into()), retry)));
}

/// Maintenance never run is due a schedule after the schedule was first seen, even if no process was
/// running at that time.
#[test]
fn maintenance_is_due_after_its_schedule_was_first_seen() {
    let engine = engine_with("first-seen", &Replay::new(), &docs_with(&[(&crate::settings::CHECK, "@daily")]));
    let collection = &engine.collections()[0];
    assert_eq!(engine.next_due(collection).map(|(_, at)| at), Some(at("2026-10-17T00:00:00Z")));
    engine.store.wait(std::time::Duration::from_hours(13), &|| false);
    assert_eq!(engine.due_next(collection, &[]).unwrap(), Some(Request::Check(vec![("Docs".into(), None)])));
}

/// With a backup schedule, a repository is stale once the schedule fired twice since its last backup.
/// Without one, after a week.
#[test]
fn a_repository_is_stale_after_two_scheduled_backups_or_a_week() {
    use crate::request::StepKey;
    let stale_at = |test: &str, docs: &str| {
        let engine = engine_with(test, &Replay::new(), docs);
        attempted(&engine, StepKey::Snapshot("Docs".into()), &backed_up(), false);
        let status = engine.status_of(&engine.collections()[0]);
        let (_, changes) = crate::clock::next_change(|| engine.status_of(&engine.collections()[0]));
        (status.primary.stale_at, changes)
    };
    let daily = docs_with(&[(&crate::settings::BACKUP, "@daily")]);
    assert_eq!(stale_at("stale-scheduled", &daily).0, Some(at("2026-10-18T00:00:00Z")));
    let week = at("2026-10-23T12:00:00Z");
    assert_eq!(stale_at("stale-unscheduled", DOCS), (Some(week), Some(week)), "and the status changes then");
}

/// A repository that is missing shows once: not again through the failures of its steps, nor those of the
/// copies that read it. Another repository's failures still show.
#[test]
fn a_missing_repository_hides_the_failures_it_causes() {
    use crate::request::StepKey;
    let missing = DOCS.replace("rest:http://backup/docs", "/nonexistent/docs");
    let engine = engine_with("missing-hides", &Replay::new(), &missing);
    attempted(&engine, StepKey::Snapshot("Docs".into()), &backed_up(), false);
    for key in [
        StepKey::Check("Docs".into()),
        StepKey::Copy("Docs".into(), "Docs-usb".into()),
        StepKey::Check("Docs-usb".into()),
    ] {
        attempted(&engine, key, &failed(), false);
    }
    let issues = engine.status_of(&engine.collections()[0]).issues;
    assert!(
        issues.iter().any(|i| matches!(i, super::report::Issue::Missing { repo, .. } if repo == "Docs")),
        "{issues:?}"
    );
    let steps: Vec<_> = issues.iter().filter_map(super::report::Issue::step).collect();
    assert_eq!(steps, [&StepKey::Check("Docs-usb".into())]);
}

/// Listed without a snapshot: never backed up. Not before the first listing, which may find some.
#[test]
fn a_listed_primary_without_snapshots_was_never_backed_up() {
    let engine = engine("never-backed-up", &Replay::new());
    let collection = &engine.collections()[0];
    let never = || engine.status_of(collection).issues.iter().any(|i| matches!(i, super::report::Issue::NeverBackedUp));
    assert!(!never());
    let inventory =
        crate::state::Inventory { listed: engine.store.now(), result: Err("offline".into()), size_outdated: false };
    engine.store.save_inventory("Docs", "Docs", &inventory).unwrap();
    assert!(never());
    // Failing instead: the failure says why.
    attempted(&engine, crate::request::StepKey::Snapshot("Docs".into()), &failed(), false);
    assert!(!never());
}

/// A notification error shows only while sends fail.
#[test]
fn an_undelivered_notification_shows_while_it_fails() {
    let engine =
        engine_with("undelivered", &Replay::new(), &docs_with(&[(&crate::settings::NOTIFY, "json://127.0.0.1:1")]));
    let collection = &engine.collections()[0];
    let undelivered = |failures| {
        let notified = crate::state::Notified { failures, error: Some("refused".into()), ..Default::default() };
        engine.store.update_notified("Docs", |_| notified.clone()).unwrap();
        engine.status_of(collection).issues.iter().any(|i| matches!(i, super::report::Issue::Undelivered { .. }))
    };
    assert!(!undelivered(0));
    assert!(undelivered(1));
}

/// An unreadable profile is a failing collection of its own, named after it.
#[test]
fn an_unreadable_profile_fails() {
    let store = Store::at(crate::sys::test_dir("engine-broken")).unwrap();
    let engine = Engine::with_profiles(
        Rustic::with_runner(Arc::new(Replay::default())),
        store,
        Runs::new(|_| {}),
        BTreeMap::new(),
        vec![("Bad".into(), "expected `=`".into())],
        Vec::new(),
        Named::default(),
    );
    let status = engine.status();
    let [bad] = status.as_slice() else { panic!("one collection") };
    assert_eq!((bad.title.as_str(), bad.primary.name.as_str(), bad.level), ("Bad", "Bad", Level::Failure));
    assert!(matches!(bad.issues.as_slice(), [super::report::Issue::Unreadable { error }] if error == "expected `=`"));
    assert_eq!(engine.report_broken(), Level::Failure);
}

/// A scheduled first failure is reported once its retry is late, and the agenda wakes then. Not after.
#[cfg(feature = "server")]
#[test]
fn the_agenda_wakes_when_a_pending_failure_is_reported() {
    use crate::request::StepKey;
    let engine = engine("agenda-pending", &Replay::new());
    let key = StepKey::Check("Docs".into());
    attempted(&engine, key.clone(), &failed(), true);
    let until = engine.store.load().last_step(&key).unwrap().pending_until().unwrap();
    let (_, changes) = crate::clock::next_change(|| engine.status_of(&engine.collections()[0]));
    assert_eq!(changes, Some(until), "the status changes then");
    assert_eq!(engine.agenda(engine.store.now()).1, Some(until));
    assert_eq!(engine.agenda(until).1, None);
    engine.store.wait(until.duration_since(engine.store.now()).unsigned_abs(), &|| false);
    let (_, changes) = crate::clock::next_change(|| engine.status_of(&engine.collections()[0]));
    assert_eq!(changes, None, "reported: nothing changes any more");
}

/// The agenda wakes when a repository becomes stale, and not after.
#[cfg(feature = "server")]
#[test]
fn the_agenda_wakes_when_a_repository_becomes_stale() {
    use crate::request::StepKey;
    let engine = engine("agenda-stale", &Replay::new());
    attempted(&engine, StepKey::Snapshot("Docs".into()), &backed_up(), false);
    let week = at("2026-10-23T12:00:00Z");
    assert_eq!(engine.agenda(engine.store.now()).1, Some(week));
    assert_eq!(engine.agenda(week).1, None);
    engine.store.wait(week.duration_since(engine.store.now()).unsigned_abs(), &|| false);
    let (_, changes) = crate::clock::next_change(|| engine.status_of(&engine.collections()[0]));
    assert_eq!(changes, None, "stale: nothing changes any more");
}

/// A queued run waits for the running ones that hold its locks, and for the same request queued before
/// it. Not for other requests queued, even with a shared lock, nor for those queued after it.
#[test]
fn a_queued_run_waits_for_its_locks_and_the_same_request_before_it() {
    use crate::result::{Doing, Status};
    let engine = engine("queued-wait", &Replay::new());
    let collections = engine.collections();
    let ask = |request: &Request| match engine.ask(&collections[0], request, Trigger::Cli).unwrap() {
        Asked::Queued(queued) => queued,
        Asked::Joined(_) => panic!("joined"),
    };
    let waits_for = |own: &crate::state::RunLog| -> Vec<Option<Status>> {
        let Doing::WaitingFor { by, .. } =
            engine.queued_wait(&engine.store.queue(Some("Docs")), &own.record, &own.request)
        else {
            panic!("not waiting")
        };
        by.into_iter().map(|h| h.status).collect()
    };
    let check = Request::Check(vec![("Docs".into(), None)]);
    let mut prune = ask(&Request::Prune(vec!["Docs".into()]));
    let mut backup = ask(&Request::Backup);
    let (copy, first, second) = (ask(&Request::Copy("Docs".into())), ask(&check), ask(&check));
    assert!(waits_for(&copy).is_empty(), "a backup queued before, which also copies");
    assert!(waits_for(&first).is_empty(), "a prune queued before, and a check after");
    assert_eq!(waits_for(&second), [Some(Status::Queued)]);
    backup.start();
    prune.start();
    assert_eq!(waits_for(&copy), [Some(Status::Running)], "the backup, not the prune");
}

/// A scheduled run starts at once: it is neither queued nor joins the same one queued. A restore asked twice runs
/// twice.
#[test]
fn a_scheduled_run_starts_without_a_queue() {
    use crate::{request::RestoreOptions, result::Status};
    let engine = engine("ask-scheduled", &Replay::new());
    let collections = engine.collections();
    let ask = |request: Request, trigger| engine.ask(&collections[0], &request, trigger).unwrap();
    let queued = ask(Request::Backup, Trigger::Cli);
    let scheduled = ask(Request::Backup, Trigger::Schedule);
    assert!(
        matches!(&scheduled, Asked::Queued(run) if run.record.status == Status::Running && run.record.id != queued.record().id)
    );
    let restore = || ask(Request::Restore(RestoreOptions::default()), Trigger::Cli);
    let first = restore();
    assert!(matches!(restore(), Asked::Queued(run) if run.record.id != first.record().id));
}

/// A result that steps running at once report alike is kept once. Failures are counted. A step that does nothing
/// more shows no activity.
#[test]
fn a_run_keeps_each_result_once_and_counts_failures() {
    use crate::{live::Guard, result::Doing, state::RunLog};
    let engine = engine("run-results", &Replay::new());
    let collections = engine.collections();
    let runs = Runs::new(|_| {});
    let guard = Guard::start(&runs, RunLog::new(&engine.store, "Docs", Request::Backup, Trigger::Cli));
    let run = super::Run::new(&collections[0], Some(guard.live.clone()));
    run.push_once(failed());
    run.push_once(failed());
    run.push_once(backed_up());
    assert_eq!(run.results_of(None), [failed(), backed_up()]);
    run.push(failed());
    assert_eq!(run.failures(), 2);
    run.activity(Some(Doing::WaitingForListing { repo: "Docs".into() }), false);
    assert_eq!(guard.live.activities().len(), 1);
    run.idle();
    assert!(guard.live.activities().is_empty());
}

/// Checks cycle through as many data subsets as set. Zero reads the structure only, and a sample takes none.
#[test]
fn checks_cycle_through_the_subsets_set() {
    use crate::settings::CHECK_SUBSETS;
    let subsets =
        |value: &str| super::maintain::subsets(&[&profile::parse("Docs", &docs_with(&[(&CHECK_SUBSETS, value)]))]);
    assert_eq!([subsets("7"), subsets("1"), subsets("0"), subsets("5%")], [Some(7), Some(1), None, None]);
}

/// A check that will not run owes the data subset it took: the next check reads it again.
#[test]
fn a_check_that_does_not_run_owes_its_subset() {
    use crate::{rustic::Reading, settings::CHECK_SUBSETS};
    let engine = engine_with("owed-subset", &Replay::new(), &docs_with(&[(&CHECK_SUBSETS, "3")]));
    let collections = engine.collections();
    let check = Request::Check(vec![("Docs".into(), None)]);
    let claimed = |engine: &Engine| {
        let Request::Check(repos) = engine.claim(&collections[0], &check).unwrap() else {
            panic!("a check claims a check")
        };
        repos[0].1
    };
    let first = claimed(&engine);
    assert_eq!(first, Some(Reading::Subset { n: 1, of: 3 }));
    engine.unread(&collections[0], &Request::Check(vec![("Docs".into(), first)]));
    assert_eq!(claimed(&engine), first);
    assert_eq!(claimed(&engine), Some(Reading::Subset { n: 2, of: 3 }));
}

/// A snapshot of `bytes` from `docs`, made at `time`, with an id of its own.
fn sized(id: u32, time: &str, bytes: u64) -> crate::rustic::Snapshot {
    serde_json::from_value(serde_json::json!({
        "id": format!("{id:08x}"),
        "time": time,
        "paths": ["/home/me/docs"],
        "hostname": "laptop",
        "original": null,
        "summary": {"total_bytes_processed": bytes},
    }))
    .unwrap()
}

/// A source shrank when an older snapshot is more than a hundred times the newest. Exactly a hundred times is not.
#[test]
fn a_source_shrank_past_a_hundredth() {
    let newest = sized(1, "2026-10-16T03:00:00Z", 1 << 20);
    let (equal, larger) = (sized(2, "2026-10-15T03:00:00Z", 100 << 20), sized(3, "2026-10-14T03:00:00Z", 101 << 20));
    let snapshots = [newest, equal, larger];
    let shrunk = super::snapshots::shrunk(&snapshots);
    let held: Vec<Vec<u32>> = shrunk
        .iter()
        .map(|(_, held)| held.iter().map(|s| u32::from_str_radix(s.id.as_str(), 16).unwrap()).collect())
        .collect();
    assert_eq!(held, [[3]]);
}

/// Of two snapshots of a source made at once, the first listed is the newest.
#[test]
fn the_first_of_two_at_once_is_the_newest() {
    let snapshots = [sized(1, "2026-10-16T03:00:00Z", 1), sized(2, "2026-10-16T03:00:00Z", 2)];
    let newest = super::snapshots::newest_by_source(&snapshots);
    let ids: Vec<&str> = newest.values().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["00000001"]);
}

/// What a step holds shows while it holds it, and no longer once it lets go.
#[test]
fn a_step_shows_what_it_holds_until_it_lets_go() {
    use crate::{live::Guard, result::Status, state::RunLog};
    let engine = engine("held", &Replay::new());
    let collections = engine.collections();
    let runs = Runs::new(|_| {});
    let guard = Guard::start(&runs, RunLog::new(&engine.store, "Docs", Request::Backup, Trigger::Cli));
    guard
        .live
        .steps(&[StepRecord { status: Status::Running, ..StepRecord::queued("snapshot to Docs".into(), Vec::new()) }]);
    let run = super::Run::new(&collections[0], Some(guard.live.clone()));
    let step = run.for_step(0);
    let using = || guard.live.record().steps[0].holds.using.clone();
    let held = engine.lock(&step, false, &[(collections[0].primary, super::holds::Use::Read)]).unwrap();
    assert_eq!(using(), ["download", "repo:Docs"]);
    drop(held);
    assert_eq!(using(), Vec::<String>::new());
}

/// A backup is of the whole collection: one of a single repository is refused.
#[test]
fn a_backup_of_one_repository_is_refused() {
    use crate::request::{RestoreOptions, RunKind};
    let engine = engine("backup-repo", &Replay::new());
    let collections = engine.collections();
    let usb = collections[0].repo("Docs-usb");
    assert!(collections[0].requests(RunKind::Backup, usb, &RestoreOptions::default()).is_err());
    assert!(collections[0].requests(RunKind::Backup, None, &RestoreOptions::default()).is_ok());
}

/// A target that a copy reached before, that was listed, or whose newest snapshot is known, is unavailable when it
/// cannot be listed: only Re-create brings it back. One never seen is created.
#[test]
fn a_target_seen_before_is_unavailable_not_created() {
    use crate::{
        request::StepKey,
        result::StepResult,
        state::{Behind, Event},
    };
    let at = at("2026-10-16T03:00:00Z");
    let copied = StepResult::InSync { target: "Docs-usb".into() };
    let copy = StepKey::Copy("Docs".into(), "Docs-usb".into());
    let seen: [(&str, Vec<Event>); 4] = [
        ("never", Vec::new()),
        ("copied", vec![Event::Attempted { key: copy, started: at, at, result: (&copied).into(), by: None }]),
        (
            "listed",
            vec![Event::Lag { collection: "Docs".into(), target: "Docs-usb".into(), at, behind: Behind::Listed(None) }],
        ),
        ("newest", vec![Event::Newest { repo: "Docs-usb".into(), at }]),
    ];
    for (name, events) in seen {
        let replay = Replay::new().ok("Docs", &["cat", "config"], &recorded("cat-config.stdout")).ok(
            "Docs",
            &["snapshots", "--json"],
            &recorded("snapshots.stdout"),
        );
        let engine = engine(&format!("copy-seen-{name}"), &replay);
        engine.store.apply(&events).unwrap();
        let collections = engine.collections();
        let report = asked(&engine, &collections[0], &Request::Copy("Docs-usb".into()));
        let created = replay.calls().iter().any(|c| c.ends_with("--init"));
        let unavailable = report.results[0].to_string().contains("Docs-usb is unavailable or deleted");
        assert_eq!((created, unavailable), (name == "never", name != "never"), "{name}: {:?}", report.results);
    }
}

/// A check that fails owes its subset: the next check reads it again. Only one that fails on its own, not as its run
/// stops, counts as failed, until that subset passes.
#[cfg(feature = "server")]
#[test]
fn a_check_stopped_owes_its_subset() {
    use crate::{live::Guard, request::Step, rustic::Reading, state::RunLog};
    for stopped in [true, false] {
        let engine = engine(&format!("check-stopped-{stopped}"), &Replay::new());
        let collections = engine.collections();
        let runs = Runs::new(|_| {});
        let guard = Guard::start(&runs, RunLog::new(&engine.store, "Docs", Request::Backup, Trigger::Cli));
        if stopped {
            guard.live.cancel();
        }
        let run = super::Run::new(&collections[0], Some(guard.live.clone()));
        let check = Step::Check { repo: "Docs".into(), reading: Reading::Subset { n: 2, of: 5 } };
        engine.maintain(&run, collections[0].primary, &check);
        let checks = engine.store.load().checks["Docs"].clone();
        assert_eq!((checks.owed.contains(&2), checks.failed.contains(&2)), (true, !stopped), "stopped: {stopped}");
    }
}

/// A forget on a repository it cannot list fails. Only a dry run, which creates no repository, has nothing to forget.
#[test]
fn a_forget_on_a_repository_it_cannot_list_fails() {
    let replay = Replay::new().ok("Docs-usb", &["cat", "config"], &recorded("cat-config.stdout"));
    let engine = engine("forget-unlisted", &replay);
    let collections = engine.collections();
    let run = super::Run::new(&collections[0], None);
    engine.forget(&run, collections[0].repo("Docs-usb").unwrap(), &[]);
    assert_eq!(run.failures(), 1, "{:?}", run.results_of(None));
}

/// A source that a hook creates learns from the one snapshot that no other source explains. With more of them than
/// such sources, the order would not tell which is whose: none learns.
#[test]
fn a_hooked_source_learns_from_the_one_snapshot_left() {
    let config = "[repository]\nrepository = '/backup'\npassword = 'x'\n\
        [[backup.snapshots]]\nsources = ['docs']\nas-path = '/docs'\n\
        [[backup.snapshots]]\nsources = ['not yet']\nhooks = { run-before = ['mount'] }\n";
    let profile = profile::parse("Docs", config);
    let snapshot = |id: u32, path: &str| -> crate::rustic::Snapshot {
        serde_json::from_value(serde_json::json!({
            "id": format!("{id:08x}"), "time": "2026-10-16T03:00:00Z", "paths": [path], "hostname": "laptop"
        }))
        .unwrap()
    };
    let (docs, mounted, other) = (snapshot(1, "/docs"), snapshot(2, "/mnt/docs"), snapshot(3, "/mnt/other"));
    let paired = |snapshots: &[crate::rustic::Snapshot]| -> Vec<Vec<String>> {
        super::backup::learned(&profile.sources, snapshots)
            .into_iter()
            .map(|(s, sn)| [s.sources.clone(), sn.paths.clone()].concat())
            .collect()
    };
    assert_eq!(paired(&[docs.clone(), mounted.clone()]), [["not yet", "/mnt/docs"]]);
    assert!(paired(&[docs.clone(), mounted, other]).is_empty(), "two left for one source");
    let unhooked = profile::parse(
        "Docs",
        "[repository]\nrepository = '/backup'\npassword = 'x'\n[[backup.snapshots]]\nsources = ['/home/me/docs']\n",
    );
    assert!(super::backup::learned(&unhooked.sources, &[docs]).is_empty(), "no source to learn");
}

/// A new computer's hooked sources learn from the newest snapshot of as many unexplained sources, in the order rustic
/// made them. With fewer sources than wanted, none learns.
#[test]
fn hooked_sources_pick_the_newest_unexplained_snapshots() {
    let snapshot = |id: u32, time: &str, path: &str| -> crate::rustic::Snapshot {
        serde_json::from_value(serde_json::json!({ "id": format!("{id:08x}"), "time": time, "paths": [path] })).unwrap()
    };
    let snapshots = [
        snapshot(1, "2026-10-14T03:00:00Z", "/mnt/old"),
        snapshot(2, "2026-10-16T03:00:00Z", "/mnt/b"),
        snapshot(3, "2026-10-15T03:00:00Z", "/mnt/a"),
        snapshot(4, "2026-10-13T03:00:00Z", "/mnt/a"),
    ];
    let ids = |wanted| {
        super::snapshots::picks(&snapshots, wanted)
            .map(|p| p.iter().map(|s| s.id.short().to_owned()).collect::<Vec<_>>())
    };
    assert_eq!(ids(2), Some(vec!["00000003".to_owned(), "00000002".to_owned()]));
    assert_eq!(ids(4), None, "three sources for four");
}
