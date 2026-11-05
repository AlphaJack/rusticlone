//! The collections' status and what it reports.

use super::*;

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

/// A scheduled step's first failure shows its last success until the failure counts.
/// Without profiles nothing is backed up: a warning on the first start, a failure once collections had run.
#[test]
fn no_profile_is_a_warning_at_first_and_a_failure_after_runs() {
    use crate::{log::Message, request::StepKey, result::StepResult, state::Event};
    let engine = engine_of("unconfigured", &Replay::new(), &[]);
    assert_eq!(engine.unconfigured(), Some(crate::engine::report::Unconfigured::New));
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
    assert!(checked() == crate::engine::report::Last::Never);
    engine.store.wait(std::time::Duration::from_mins(6), &|| false);
    assert!(checked() == crate::engine::report::Last::Failing(1));
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

/// Metrics name an issue's kind as `/status` does.
#[cfg(feature = "server")]
#[test]
fn issue_kinds_match_their_json() {
    let issues = [
        crate::engine::Issue::NeverBackedUp,
        crate::engine::Issue::Missing { repo: "Docs".into(), primary: true },
        crate::engine::Issue::Unreadable { error: String::new() },
    ];
    for issue in issues {
        assert_eq!(serde_json::to_value(&issue).unwrap()["kind"], <&str>::from(&issue));
    }
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
        issues.iter().any(|i| matches!(i, crate::engine::report::Issue::Missing { repo, .. } if repo == "Docs")),
        "{issues:?}"
    );
    let steps: Vec<_> = issues.iter().filter_map(crate::engine::report::Issue::step).collect();
    assert_eq!(steps, [&StepKey::Check("Docs-usb".into())]);
}

/// Listed without a snapshot: never backed up. Not before the first listing, which may find some.
#[test]
fn a_listed_primary_without_snapshots_was_never_backed_up() {
    let engine = engine("never-backed-up", &Replay::new());
    let collection = &engine.collections()[0];
    let never =
        || engine.status_of(collection).issues.iter().any(|i| matches!(i, crate::engine::report::Issue::NeverBackedUp));
    assert!(!never());
    let inventory =
        crate::state::Inventory { listed: engine.store.now(), result: Err("offline".into()), size_outdated: false };
    engine.store.save_inventory("Docs", "Docs", &inventory).unwrap();
    assert!(never());
    // Failing instead: the failure says why.
    attempted(&engine, crate::request::StepKey::Snapshot("Docs".into()), &failed(), false);
    assert!(!never());
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
    assert!(
        matches!(bad.issues.as_slice(), [crate::engine::report::Issue::Unreadable { error }] if error == "expected `=`")
    );
    assert_eq!(engine.report_broken(), Level::Failure);
}

/// A refresh reports each repository it cannot list, but for one not created yet, as a copy target before its
/// first copy: its status says so already.
#[test]
fn a_refresh_reports_no_target_not_created_yet() {
    let usb = USB.replace("rest:http://usb/docs", "/nowhere/usb");
    let engine = engine_of("refresh-not-created", &Replay::new(), &[("Docs", DOCS), ("Docs-usb", &usb)]);
    let collections = engine.collections();
    let report = engine.refresh(&collections[0]);
    let kinds: Vec<String> =
        report.results.iter().map(|r| serde_json::to_value(r).unwrap()["kind"].as_str().unwrap().to_owned()).collect();
    assert_eq!(kinds, ["primary-unavailable"], "{:?}", report.results);
}
