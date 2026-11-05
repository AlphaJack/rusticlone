//! Runs that the server starts, shows and stops.

use super::*;

#[test]
fn server_runs_due_jobs_and_serves_status() {
    let sb = docs_with_remote("");
    sb.common_env(&[("BACKUP_CRON", "0 0 * * *")]);
    let server = sb.server(&[]);
    // Overdue at start: backup and copy run on the first tick.
    wait_for(60, "backup and copy", || {
        let profile = server.collection("Docs");
        let target = &profile["targets"][0];
        profile["snapshot"]["last_ok"].is_string()
            && target["lag"]["checked_at"].is_string()
            && target["lag"]["behind_since"].is_null()
    });
    let page = server.get("/");
    assert!(page.code == 200 && page.body.contains("Docs"), "{}", page.body);
    assert_eq!(server.collection("Docs")["targets"][0]["status"]["kind"], "in-sync");
    drop(server);
    assert_eq!(sb.snapshots("Docs-remote").len(), 1);
}

#[test]
fn runs_are_kept_with_their_log() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let server = sb.server(&[]);
    // One run, with a step per repository, which the page opens.
    assert_eq!(server.post_form("/collections/Docs/runs", "kind=check").code, 201);
    wait_for(60, "the checks", || server.collection("Docs")["targets"][0]["check"]["last_ok"].is_string());
    let records = || sb.store().runs("Docs", usize::MAX);
    let checks = || records().into_iter().filter(|r| json!(r.kind) == "check" && r.finished.is_some()).count();
    wait_for(10, "the run to be recorded", || checks() == 1);
    let scopes: Vec<_> = records().iter().map(|r| (json!(r.kind), json!(r.trigger), r.scope.clone())).collect();
    assert_eq!(scopes, [(json!("check"), json!("web"), None), (json!("backup"), json!("cli"), None)]);
    let runs = card(&server.get("/collections/Docs").body, "runs");
    assert!(runs.contains(">web<") && runs.contains(">cli<"), "{runs}");
    // Newest first: the check, whose log the collection page shows. Picking another run replaces it.
    let table = runs.get(runs.find("<table").unwrap()..).unwrap();
    let ids: Vec<&str> =
        table.split(r#"href="/collections/Docs/runs/"#).skip(1).map(|s| s.split('#').next().unwrap()).collect();
    let log = card(&server.get("/collections/Docs").body, "log");
    assert!(log.contains("[Docs-remote] rustic check"), "{log}");
    let verified = |id: &str| {
        let run = records().into_iter().find(|r| r.id.to_string() == id).unwrap();
        run.all_results().any(|l| l.values["kind"] == "verified")
    };
    assert!(verified(ids[0]), "{ids:?}");
    // With its rustic commands, their durations and exits. A directory with a space, as on macOS, is quoted.
    let dir = sb.profile_dir().display().to_string();
    assert!(
        log.contains(
            "-P Docs-remote --json-progress --progress-interval 1s check --read-data --read-data-subset 1/10</code>"
        ) && [format!("cd {dir}"), format!("cd &#39;{dir}&#39;")].iter().any(|cd| log.contains(cd))
            && log.contains(r#"<span class="dot success""#),
        "{log}"
    );
    // Its commands and log lines tell which step made them: the primary's check and the target's, which run at
    // the same time, so in either order.
    assert!(
        [0, 1].iter().all(|step| log.contains(&format!(r#"<tr data-step="{step}">"#))
            && log.contains(&format!(r#"" data-step="{step}">"#))
            && log.contains(&format!(r#"<span data-step="{step}" data-call="#))),
        "{log}"
    );
    let picked = card(&server.get(&format!("/collections/Docs/runs/{}", ids[1])).body, "log");
    assert!(picked.contains("[Docs] rustic backup"), "{picked}");
    // Run ids are times: no run has 1, and its page is not another run's.
    assert_eq!(server.get("/collections/Docs/runs/1").code, 404);
    let page = server.get("/collections/Docs").body;
    assert!(page.matches(r#"id="log""#).count() == 1, "{page}");
    // The overview's runs link to them.
    let overview = card(&server.get("/").body, "recent-runs");
    let id = overview.split("/collections/Docs/runs/").nth(1).unwrap().split('#').next().unwrap();
    assert!(verified(id), "{id}");
}

#[test]
fn a_stopped_step_lets_the_run_go_on() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 60, "[forget]\nkeep-last = 1\n");
    // Created first, so that the forget always finds the repository, however soon the snapshot stops.
    assert!(sb.rustic(&["-P", "Slow", "init"]).status.success());
    let server = sb.server(&[]);
    server.post_form("/collections/Slow/runs", "kind=backup");
    wait_for(30, "the backup", || server.collection("Slow")["running"][0]["activities"][0]["on"][0] == "Slow");
    let id = server.collection("Slow")["running"][0]["id"].clone();
    assert_eq!(server.delete(&format!("/collections/Slow/runs/{id}/steps/0")).code, 200);
    // Under the load of the whole suite, stopping rustic and running the forget can take a while; under 60
    // seconds, when the snapshot would end on its own.
    wait_for(45, "the run to end", || server.collection("Slow")["running"] == serde_json::json!([]));
    // The snapshot stopped; the forget, which does not need it, still ran (with nothing to forget), and
    // so did the run.
    let run = &sb.store().runs("Slow", 1)[0];
    let steps: Vec<_> = run.steps.iter().map(|s| serde_json::json!(s.status)).collect();
    assert_eq!(steps, ["cancelled", "succeeded"]);
    assert!(
        serde_json::json!(run.status) == "succeeded-with-warnings"
            && !run.all_results().any(|l| l.values["kind"] == "unmet")
    );
}

#[test]
fn cancelled_backup_waits_for_its_next_schedule() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 60, KEEP);
    sb.backup_profile("Fast", &[&["fast"]], &[], "");
    sb.write("fast/a.txt", "a");
    sb.common_env(&[("BACKUP_CRON", "@daily")]);
    let server = sb.server(&[]);
    wait_for(30, "the scheduled backup", || {
        server.collection("Slow")["running"][0]["activities"][0]["on"][0] == "Slow"
    });
    assert!(server.get("/collections/Slow").body.contains(r#"class="progress""#));
    // The running backup shows on the repository it writes to, and can be stopped.
    let repos = card(&server.get("/collections/Slow").body, "repos");
    assert!(repos.contains(r#"class="spinner""#), "{repos}");
    // Requested meanwhile: each request is its own run, and can be stopped while waiting. The prune waits
    // for the repository the backup writes to; the check waits only on a disk that may spin.
    wait_for(20, "the repository", || sb.path("local/Slow/config").is_file());
    assert!(
        server.post_form("/collections/Slow/repos/Slow/runs", "kind=check").body.contains(r#"class="flash success""#)
    );
    server.post_form("/collections/Slow/repos/Slow/runs", "kind=prune");
    let running = || server.collection("Slow")["running"].as_array().unwrap().clone();
    wait_for(20, "the prune to wait", || {
        running().iter().any(|r| r["kind"] == "prune" && r["activities"][0]["doing"]["kind"] == "waiting-for")
    });
    for run in running().iter().filter(|r| r["kind"] != "backup") {
        // On a solid-state disk the check does not wait, and may end before it is stopped.
        let code = server.delete(&format!("/collections/Slow/runs/{}", run["id"])).code;
        assert!(code == 200 || running().iter().all(|r| r["id"] != run["id"]), "{code} for {run}");
    }
    wait_for(10, "them to stop", || running().len() == 1);
    let stopped = Instant::now();
    assert_eq!(server.delete("/collections/Slow/runs").code, 200);
    wait_for(20, "the backup to stop", || server.collection("Slow")["running"] == serde_json::json!([]));
    assert!(stopped.elapsed() < Duration::from_secs(20), "rustic was not killed");
    let profile = server.collection("Slow");
    assert!(profile["snapshot"]["cancelled"] == true && profile["snapshot"]["last_ok"].is_null(), "{profile}");
    // Not a issue: the user requested it.
    assert!(!profile["issues"].as_array().unwrap().iter().any(|i| i["kind"] == "failed"), "{profile}");
    let runs = sb.store().runs("Slow", usize::MAX);
    assert!(runs.iter().any(|r| json!(r.trigger) == "schedule" && r.status == Status::Cancelled), "{profile}");
    // What rustic started (the stdin command) is gone too: it would hold the backup's locks.
    wait_for(20, "Fast's backup", || server.collection("Fast")["snapshot"]["last_ok"].is_string());
    drop(server);

    // Not due again until the schedule fires after the cancellation.
    sb.run(&["due", "--collection", "Slow"]).assert_exit(0).assert_no_output("rustic backup");
    assert!(sb.snapshots("Slow").is_empty());
}

#[test]
fn a_command_line_run_is_followed_live() {
    let (_sb, server, mut cli) = backing_up_elsewhere();
    wait_for(10, "its log", || card(&server.get("/collections/Slow").body, "log").contains("[Slow] rustic backup"));
    assert!(card(&server.get("/collections/Slow").body, "runs").contains(r#"class="spinner""#));
    assert!(cli.wait().unwrap().success());
    assert_eq!(server.collection("Slow")["running"], serde_json::json!([]));
    let runs = card(&server.get("/collections/Slow").body, "runs");
    assert!(runs.contains(">cli<") && !runs.contains(r#"class="spinner""#), "{runs}");
}

#[test]
fn server_shows_a_running_backup_with_progress() {
    let sb = docs_with_remote("");
    // Data that rustic takes a while over: it neither compresses nor deduplicates.
    for i in 0..8 {
        fs::write(sb.path(&format!("source/docs/big{i}.bin")), noise(i, 12 << 20)).unwrap();
    }
    sb.common_env(&[("BACKUP_CRON", "0 0 * * *")]);
    let server = sb.server(&[]);
    // Seen together while it runs, as the backup may end at any time: its progress, the step that the recent
    // runs on the home page name, the collection page's progress bar, and its commands so far.
    wait_for(60, "progress", || {
        let profile = server.collection("Docs");
        assert!(profile["snapshot"]["last_ok"].is_null(), "the backup finished without progress showing up: {profile}");
        let seen = &profile["running"][0]["activities"][0];
        let (home, page) = (server.get("/").body, server.get("/collections/Docs").body);
        let (home, log) =
            (shown_card(&home, "recent-runs").unwrap_or_default(), shown_card(&page, "log").unwrap_or_default());
        let steps = sb.store().runs("Docs", 1).first().map(|r| r.steps.clone()).unwrap_or_default();
        seen["progress"]["percent_done"].is_number()
            && ["backing up", "copying"].iter().any(|verb| seen["on"][1] == *verb)
            && steps.iter().enumerate().any(|(i, s)| home.contains(&format!(">{}. {}<", i + 1, s.label)))
            && page.contains("class=\"progress\"")
            && log.contains("[Docs] rustic ")
            && log.contains(r#"<tr data-call="0""#)
    });
    // Once done, the activity is gone, and no rustic keeps running after the server stops.
    wait_for(60, "the backup to finish", || {
        let profile = server.collection("Docs");
        profile["running"] == serde_json::json!([]) && profile["targets"][0]["lag"]["checked_at"].is_string()
    });
}

/// Auto-update set to neither `true` nor `false` stops the server from starting, rather than guessing.
#[cfg(feature = "self-update")]
#[test]
#[expect(clippy::disallowed_methods, reason = "observe external processes over a bounded interval")]
fn a_server_with_an_invalid_auto_update_does_not_start() {
    let sb = Sandbox::new();
    let (listen, update) = (var("LISTEN"), var("AUTO_UPDATE"));
    let mut server = sb.spawn(&["serve"], &[(listen.as_str(), "127.0.0.1:0"), (update.as_str(), "maybe")]);
    let deadline = Instant::now() + Duration::from_secs(30);
    while server.try_wait().unwrap().is_none() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
    }
    let ended = server.try_wait().unwrap();
    _ = server.kill();
    assert_eq!(ended.and_then(|s| s.code()), Some(1));
}

/// With nothing due, the server is idle: reacting to state changes does not change the state.
#[test]
#[expect(clippy::disallowed_methods, reason = "observe external processes over a bounded interval")]
fn an_idle_server_leaves_its_state_alone() {
    let sb = Sandbox::new();
    let notify = global_env(&[("NOTIFY_URLS", "json://127.0.0.1:1")]);
    sb.backup_profile("Docs", &[&["docs"]], &[], &notify);
    sb.write("docs/a.txt", "a");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.common_env(&[("BACKUP_CRON", "@daily"), ("CHECK_CRON", "@weekly")]);
    let server = sb.server(&[]);
    let db = sb.path("store/state.redb");
    let modified = || fs::metadata(&db).and_then(|m| m.modified()).unwrap();
    thread::sleep(Duration::from_secs(2));
    let before = modified();
    thread::sleep(Duration::from_secs(2));
    assert_eq!(modified(), before, "written while idle: {}", server.output());
}

/// A notification that cannot be sent is retried with backoff, not at every wake of the server, and shows on
/// its collection.
#[test]
#[expect(clippy::disallowed_methods, reason = "observe external processes over a bounded interval")]
fn a_failed_notification_waits_before_retrying() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &[], &global_env(&[("NOTIFY_URLS", "json://127.0.0.1:1")]));
    // No sources: the backup fails, and is reported from its second failure.
    sb.run(&["backup", "--all"]);
    sb.run(&["backup", "--all"]);
    sb.common_env(&[("BACKUP_CRON", "@daily"), ("CHECK_CRON", "@weekly")]);
    let server = sb.server(&[]);
    thread::sleep(Duration::from_secs(3));
    let attempts = server.output().matches(r#""kind":"undelivered""#).count();
    assert!(attempts <= 1, "{attempts} attempts: {}", server.output());
    assert!(json_has(&server.collection("Docs")["issues"].to_string(), &json!({"kind": "undelivered"})));
}

/// A run of the server is stopped by its id, which says so. Stopping nothing says nothing.
#[test]
fn a_run_is_stopped_by_its_id() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 30, "");
    let server = sb.server(&[]);
    let cancelled = json!({"event": {"kind": "cancelled-from-web"}});
    assert_eq!(server.delete("/collections/Slow/runs").code, 200);
    assert!(!server.logged(&cancelled), "nothing was stopped: {}", server.output());
    server.post_form("/collections/Slow/runs", "kind=backup");
    wait_for(20, "the backup", || server.collection("Slow")["running"][0]["activities"][0]["on"][0] == "Slow");
    let id = server.collection("Slow")["running"][0]["id"].clone();
    assert_eq!(server.delete(&format!("/collections/Slow/runs/{id}")).code, 200);
    wait_for(20, "it to stop", || server.collection("Slow")["running"] == json!([]));
    assert!(server.logged(&cancelled), "{}", server.output());
}
