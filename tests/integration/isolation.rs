use std::{
    fs, thread,
    time::{Duration, Instant},
};

use serde_json::json;

use rusticlone::Status;

use crate::common::{NAME, Sandbox, docs_with_remote, signal, slow_profile, to_run, var, wait_for};

#[test]
fn stray_rustic_environment_does_not_redirect_backups() {
    let sb = docs_with_remote("");
    let (elsewhere, cache) = (sb.path("elsewhere"), sb.path("env-cache"));
    let envs = [
        ("RUSTIC_REPOSITORY", elsewhere.to_str().unwrap()),
        ("RUSTIC_DRY_RUN", "true"),
        ("RUSTIC_CACHE_DIR", cache.to_str().unwrap()),
    ];
    sb.run_with(&["backup", "--all"], &envs).assert_exit(0);
    assert!(!elsewhere.exists());
    assert_eq!(sb.snapshots("Docs").len(), 1);
    // The rest, as a container's password or cache, reach rustic.
    assert!(cache.exists());
}

/// A disaster recovery runs from the folder that kept the profiles.
#[test]
fn profiles_in_the_working_directory_are_read() {
    let sb = docs_with_remote("");
    sb.write("kept-source/a.txt", "a");
    fs::write(
        sb.path("cwd/Kept.toml"),
        format!(
            "[repository]\nrepository = '{}'\npassword = 'x'\n[[backup.snapshots]]\nsources = ['{}']\n",
            sb.path("kept").display(),
            sb.path("source/kept-source").display()
        ),
    )
    .unwrap();
    sb.run(&["collections"]).assert_result("backup-profile", json!({"name": "Kept"}));
    sb.run(&["backup", "--collection", "Kept"]).assert_exit(0);
    assert_eq!(sb.snapshots("Kept").len(), 1);
}

#[test]
fn profiles_are_classified() {
    let sb = docs_with_remote("");
    sb.profile("broken", "[repository\n");
    // A dry run would report backups it did not make.
    sb.profile("dry", "[global]\nuse-profiles = ['common']\ndry-run = true\n");
    sb.run(&["collections"])
        .assert_exit(1)
        .assert_result("backup-profile", json!({"name": "Docs"}))
        .assert_result("copy-target", json!({"name": "Docs-remote", "copy_target_of": ["Docs"]}))
        .assert_result("fragment", json!({"name": "common"}))
        .assert_result("broken", json!({"name": "broken"}))
        .assert_result("broken", json!({"name": "dry"}));
    // Its repository is not inspected: unknown, not in sync.
    let status = sb.run(&["collections"]).stdout;
    let docs = status.lines().filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok());
    let status = docs.into_iter().find(|e| e.get("collections").is_some()).unwrap();
    let broken = status["collections"].as_array().unwrap().iter().find(|p| p["primary"]["name"] == "broken").unwrap();
    assert_eq!(broken["primary"]["status"]["kind"], "unknown", "{broken}");
    // Selecting collections skips the broken one.
    sb.run(&["backup", "--collection", "Docs"]).assert_exit(0);
}

#[test]
fn a_name_that_matches_no_collection_fails() {
    let sb = docs_with_remote("");
    for command in ["due", "notify", "collections", "backup"] {
        sb.run(&[command, "--collection", "Nope"]).assert_exit(1).assert_output("Nope is not a collection");
    }
}

#[test]
fn concurrent_invocations_are_serialised() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-remote"], "[forget]\nkeep-last = 2\n");
    sb.target("Docs-remote", "Docs", "");
    sb.write("docs/a.txt", "a");
    sb.common_env(&[("BACKUP_CRON", "* * * * *")]);
    // Read at once: a child blocked on a full pipe would keep its locks from the others.
    let outputs: Vec<_> = ["backup", "backup", "backup", "due"]
        .iter()
        .map(|cmd| sb.spawn(&[cmd, "--all"], &[]))
        .map(|child| thread::spawn(|| child.wait_with_output().unwrap()))
        .collect();
    for output in outputs {
        to_run(&output.join().unwrap()).assert_no_json(&json!({"level": "ERROR"}));
    }
    // Not every invocation only waited.
    assert!(!sb.snapshots("Docs").is_empty());
    sb.common_env(&[("CHECK_SUBSETS", "1")]);
    sb.run(&["check", "--all"]).assert_exit(0);
}

#[test]
fn json_logs_are_one_object_per_line() {
    let sb = docs_with_remote("");
    let run = sb.run(&["backup", "--all"]);
    run.assert_exit(0);
    let events: Vec<serde_json::Value> =
        run.stdout.lines().map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON: {l}: {e}"))).collect();
    // Each result's kind and values are its `event`, next to its text.
    let report = |event: serde_json::Value| {
        events.iter().any(|e| {
            e["target"] == NAME
                && e["collection"] == "Docs"
                && e["level"] == "INFO"
                && e["message"].is_string()
                && crate::common::has(&e["event"], &event)
        })
    };
    let copied = json!({"kind": "copied", "target": "Docs-remote", "snapshots": 1});
    assert!(report(json!({"kind": "backed-up"})) && report(copied), "{}", run.stdout);
    assert!(events.iter().all(|e| e["timestamp"].is_string() && e["target"] != "rustic"));
    // Rustic's own log is on stderr, with the profile it ran for.
    assert!(crate::common::json_has(&run.output, &json!({"target": "rustic", "profile": "Docs-remote"})));

    // `collections` prints one document with the whole status.
    let status: serde_json::Value = serde_json::from_str(&sb.run(&["collections"]).stdout).unwrap();
    assert!(status["collections"][0]["primary"]["name"] == "Docs", "{status}");
}

/// A collection whose backup reads a stdin command that sleeps, so rustic prints nothing meanwhile.
#[test]
fn silent_rustic_is_killed() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Sleepy", 10, "");
    let envs: &[(&str, &str)] = &[(&var("STALL_TIMEOUT"), "1s")];
    sb.run_with(&["backup", "--all"], envs).assert_exit(1).assert_result("failed", json!({}));
}

#[test]
fn a_killed_run_leaves_its_locks_with_rustic() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Sleepy", 4, "");
    let mut first = sb.spawn(&["backup", "--all"], &[]);
    wait_for(10, "rustic to start", || {
        let store = sb.store();
        let started = |id| store.run_log_tail("Sleepy", id, None).0.iter().any(|l| l.text.contains("rustic backup"));
        store.unfinished(Some("Sleepy")).iter().any(|r| started(r.id))
    });
    first.kill().unwrap();
    first.wait().unwrap();
    // Rustic still runs and holds the collection's lock. Windows releases locks with the process that took
    // them: there, rustic ended with it.
    #[cfg(not(windows))]
    {
        sb.common_env(&[("BACKUP_CRON", "@daily")]);
        sb.run(&["due", "--all"]).assert_exit(0).assert_result("running-already", json!({}));
    }
    // Once it ends, the collection runs again, and quickly: the parent snapshot, if any, is not read.
    sb.kill_leftovers();
    slow_profile(&sb, "Sleepy", 0, "");
    sb.run(&["backup", "--all"]).assert_exit(0);
    // The interrupted run's log was saved as it went, and the run stays unfinished.
    let (store, records) = (sb.store(), sb.store().runs("Sleepy", 2));
    assert!(records[1].finished.is_none() && records[0].finished.is_some());
    assert!(store.run_log_tail("Sleepy", records[1].id, None).0.iter().any(|l| l.text.contains("rustic backup")));
}

#[test]
fn interrupting_stops_rustic_and_what_it_started() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 60, "");
    let first = sb.spawn(&["backup", "--collection", "Slow"], &[]);
    wait_for(30, "the backup to start", || {
        let store = sb.store();
        let running = |r: &rusticlone::RunRecord| r.steps.iter().any(|s| s.status == Status::Running);
        store.unfinished(Some("Slow")).iter().any(|r| store.alive(r) && running(r))
    });
    // Ctrl-C reaches only our process, because rustic has its own process group.
    assert!(signal(first.id(), "INT"));
    let interrupted = Instant::now();
    to_run(&first.wait_with_output().unwrap()).assert_exit(130).assert_json(&json!({"stopped": true}));
    assert!(interrupted.elapsed() < Duration::from_secs(10));
    // The stdin command, which also held the backup's locks, is gone.
    sb.backup_profile("Fast", &[&["fast"]], &[], "");
    sb.write("fast/a.txt", "a");
    let started = Instant::now();
    sb.run(&["backup", "--collection", "Fast"]).assert_exit(0);
    assert!(started.elapsed() < Duration::from_secs(20), "waited for the slow backup's leftovers");
}

/// Asked while the same backup runs and another is queued, a command joins the queued one: it waits for
/// it, shows whom it waits for, and reports its results instead of backing up a third time.
#[test]
fn a_backup_joins_the_same_one_queued() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Sleepy", 3, "");
    let unfinished = |status| {
        let store = sb.store();
        store.unfinished(Some("Sleepy")).iter().any(|r| r.status == status && store.alive(r))
    };
    let running = sb.spawn(&["backup", "--all"], &[]);
    wait_for(10, "the first backup", || unfinished(Status::Running));
    let queued = sb.spawn(&["backup", "--all"], &[]);
    wait_for(10, "the second to queue", || unfinished(Status::Queued));
    sb.run(&["backup", "--all"])
        .assert_exit(0)
        .assert_json(&json!({"status": "queued", "locks": ["backup collection"]}))
        .assert_result("joined", json!({"label": "backup collection"}));
    assert!(
        running.wait_with_output().unwrap().status.success() && queued.wait_with_output().unwrap().status.success()
    );
    assert_eq!(sb.snapshots("Sleepy").len(), 2);
}
