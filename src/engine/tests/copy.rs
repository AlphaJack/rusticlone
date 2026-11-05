//! Copies, and the forgets that wait for them.

use super::*;

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
    assert_eq!(ids(crate::engine::snapshots::own(&collection(""), &[laptop(), desktop()])), ["a0000000"]);
    // A source's own `host` selects its group, even if older.
    let own_host = profile::parse("Docs", &DOCS.replace("as-path", "host = \"laptop\"\nas-path"));
    assert_eq!(ids(crate::engine::snapshots::own(&own_host, &[laptop(), desktop()])), ["a0000000"]);
    // Reinstalled with another host name: the newest group, to restore from.
    assert_eq!(ids(crate::engine::snapshots::own(&profile::parse("Docs", DOCS), &[laptop(), desktop()])), ["b0000000"]);
    // A `group-by` without `host`: every host's snapshots, as configured.
    let together = group(None, &[("a", "2026-10-01T00:00:00Z"), ("b", "2026-10-02T00:00:00Z")]);
    assert_eq!(
        ids(crate::engine::snapshots::own(&collection("[global]\ngroup-by = \"paths\"\n"), &[together])),
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
        crate::engine::shrunk(snapshots)
            .into_iter()
            .flat_map(|(_, held)| held)
            .map(|s| s.id.short().to_owned())
            .collect()
    };
    assert_eq!(held(&[full(), snapshot("b", "2026-10-02T00:00:00Z", 1024)]), ["a0000000"]);
    // Half the size: the sources changed, nothing is held.
    assert!(held(&[full(), snapshot("b", "2026-10-02T00:00:00Z", 5 << 20)]).is_empty());
    // Too small to tell.
    assert!(held(&[snapshot("a", "2026-10-01T00:00:00Z", 1000), snapshot("b", "2026-10-02T00:00:00Z", 0)]).is_empty());
}

/// A copy into a repository fails when its listing then lacks a snapshot it copied.
#[test]
fn a_copy_into_fails_without_what_it_copied() {
    let replay = Replay::new()
        .ok("Docs", &["cat", "config"], &recorded("cat-config.stdout"))
        .ok("Docs-usb", &["cat", "config"], &recorded("cat-config.stdout"))
        .ok("Docs-usb", &["copy"], "");
    // Listed, then again after each wait for a remote that lags.
    for _ in 0..4 {
        replay.ok("Docs", &["snapshots", "--json"], &recorded("snapshots.stdout"));
    }
    let engine = engine("copy-into-unlisted", &replay);
    let collections = engine.collections();
    let run = crate::engine::Run::new(&collections[0], None);
    let missing: crate::rustic::Snapshot = serde_json::from_value(serde_json::json!({
        "id": "ffffffff", "time": "2026-10-16T03:00:00Z", "paths": ["/elsewhere"]
    }))
    .unwrap();
    let (docs, usb) = (collections[0].primary, collections[0].repo("Docs-usb").unwrap());
    let copied = engine.copy_into(&run, docs, usb, &[missing]);
    let not_copied = copied.as_ref().err().and_then(|e| e.downcast_ref::<crate::engine::Error>());
    assert!(matches!(not_copied, Some(crate::engine::Error::NotCopiedInto { .. })), "{copied:?}");
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
    let shrunk = crate::engine::snapshots::shrunk(&snapshots);
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
    let newest = crate::engine::snapshots::newest_by_source(&snapshots);
    let ids: Vec<&str> = newest.values().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["00000001"]);
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

/// A forget on a repository it cannot list fails. Only a dry run, which creates no repository, has nothing to forget.
#[test]
fn a_forget_on_a_repository_it_cannot_list_fails() {
    let replay = Replay::new().ok("Docs-usb", &["cat", "config"], &recorded("cat-config.stdout"));
    let engine = engine("forget-unlisted", &replay);
    let collections = engine.collections();
    let run = crate::engine::Run::new(&collections[0], None);
    engine.forget(&run, collections[0].repo("Docs-usb").unwrap(), &[]);
    assert_eq!(run.failures(), 1, "{:?}", run.results_of(None));
}

/// The primary's forget keeps what no copy has reached yet.
#[test]
fn a_primary_forget_holds_what_no_copy_reached() {
    use crate::request::Step;
    let replay = Replay::new()
        .ok("Docs", &["cat", "config"], &recorded("cat-config.stdout"))
        .ok("Docs", &["snapshots", "--json"], &recorded("snapshots.stdout"))
        .ok("Docs", &["snapshots", "--json"], &recorded("snapshots.stdout"))
        .ok("Docs", &["forget", "--json"], &recorded("forget.stdout"));
    let engine = engine("forget-primary-held", &replay);
    let collections = engine.collections();
    let run = crate::engine::Run::new(&collections[0], None);
    // A copy confirmed, that holds none of the primary's groups.
    run.shared.copied.lock().insert("Docs-usb".into(), BTreeMap::new());
    engine.run_step(&run, &Step::Forget { repo: "Docs".into() });
    let calls = replay.calls();
    assert!(calls.iter().any(|c| c.contains(" forget ") && c.contains("--keep-id")), "{calls:?}");
}

/// A run that stops creates no target: a target it cannot list then may only be out of reach for a moment.
#[cfg(feature = "server")]
#[test]
fn a_stopped_copy_creates_no_target() {
    use crate::{live::Guard, state::RunLog};
    let replay = Replay::new().ok("Docs", &["cat", "config"], &recorded("cat-config.stdout")).ok(
        "Docs",
        &["snapshots", "--json"],
        &recorded("snapshots.stdout"),
    );
    let engine = engine("copy-stopped", &replay);
    let collections = engine.collections();
    let runs = Runs::new(|_| {});
    let guard = Guard::start(&runs, RunLog::new(&engine.store, "Docs", Request::Copy("Docs-usb".into()), Trigger::Cli));
    guard.live.cancel();
    let run = crate::engine::Run::new(&collections[0], Some(guard.live.clone()));
    engine.copy_step(&run, collections[0].repo("Docs-usb").unwrap());
    let calls = replay.calls();
    assert!(calls.iter().any(|c| c.contains("Docs-usb snapshots")), "it tried to list it: {calls:?}");
    assert!(!calls.iter().any(|c| c.ends_with("--init")), "{calls:?}");
}
