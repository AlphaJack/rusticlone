//! Runs that wait for each other: locks, disks and the queue.

use super::*;

/// On another disk, a check does not wait for a backup.
#[test]
fn server_runs_jobs_independently() {
    let sb = Sandbox::new();
    let Some(elsewhere) = other_disk(&sb.path("")) else {
        eprintln!("skipped: no other disk to put a repository on");
        return;
    };
    let repo = elsewhere.join(NAME).join(format!("{}-fast", std::process::id()));
    _ = fs::remove_dir_all(&repo);
    sb.backup_profile_in("Fast", repo.to_str().unwrap(), &[&["fast"]], &[], &global_env(&[("CHECK_CRON", "@daily")]));
    sb.write("fast/a.txt", "a");
    sb.run(&["backup", "--collection", "Fast"]).assert_exit(0);
    // Slow's backup goes on until the test lets it end, however long Fast's check takes.
    let release = sb.path("release");
    stdin_profile(&sb, "Slow", &cmd::until_exists(&release), &global_env(&[("BACKUP_CRON", "@daily")]));
    sb.scheduled_long_ago(&["check:Fast"]);
    let server = sb.server(&[]);
    let slow_runs = || server.collection("Slow")["running"][0]["activities"][0].is_object();
    wait_for(30, "Slow's backup to start", slow_runs);
    // Fast's check does not wait for Slow's backup.
    wait_for(30, "Fast's check", || server.collection("Fast")["primary"]["check"]["last_ok"].is_string());
    assert!(slow_runs(), "Slow should still be running");
    fs::write(&release, "").unwrap();
    wait_for(60, "Slow's backup", || server.collection("Slow")["snapshot"]["last_ok"].is_string());
    fs::remove_dir_all(&repo).unwrap();
    _ = fs::remove_dir(repo.parent().unwrap());
}

/// A check shares the repository with a backup, but not the disk: they run one at a time. The
/// sandbox's tmpfs is not reported as solid-state, so it counts as a disk that may spin. macOS reports
/// its disk as solid-state, where both run at once.
#[cfg(target_os = "linux")]
#[test]
fn a_check_waits_for_a_backup_on_the_same_disk() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 5, "");
    assert!(sb.rustic(&["-P", "Slow", "init"]).status.success());
    let server = sb.server(&[]);
    server.post_form("/collections/Slow/runs", "kind=backup");
    wait_for(20, "the backup", || server.collection("Slow")["running"][0]["activities"][0]["on"][0] == "Slow");
    server.post_form("/collections/Slow/repos/Slow/runs", "kind=check");
    let waiting = || server.collection("Slow")["running"].to_string().contains(r#""locks":["disk:"#);
    wait_for(30, "the check to wait", waiting);
    // The run that holds the disk shows the same name in both columns.
    let holds = || {
        server.collection("Slow")["running"].as_array().unwrap().iter().map(|r| r["holds"].clone()).collect::<Vec<_>>()
    };
    wait_for(10, "what the check waits for", || holds().iter().any(|h| h["waiting_for"].is_string()));
    let holds = holds();
    let disk = holds.iter().find_map(|h| h["waiting_for"].as_str()).unwrap().to_owned();
    assert!(
        disk.starts_with("disk:/") && holds.iter().any(|h| h["using"].as_array().unwrap().iter().any(|u| *u == disk)),
        "{holds:?}"
    );
    let runs = card(&server.get("/collections/Slow").body, "runs");
    assert!(runs.matches(&format!(">{disk}<")).count() == 2, "{runs}");
    // The waiting run links to the holding one.
    let backup =
        server.collection("Slow")["running"].as_array().unwrap().iter().find(|r| r["kind"] == "backup").unwrap()["id"]
            .clone();
    assert!(runs.contains(&format!(r#"<a class="tag" href="/collections/Slow/runs/{backup}#log""#)), "{runs}");
    wait_for(60, "the check and the backup", || sb.store().runs("Slow", 2).iter().all(|r| r.finished.is_some()));
    // After the backup wrote. Listing and measuring afterwards do not take the disk.
    let runs = sb.store().runs("Slow", 2);
    let (check, backup) = (&runs[0], &runs[1]);
    let call = |args: &str| backup.calls.iter().find(|c| c.args.ends_with(args)).unwrap();
    let wrote = call("backup --json");
    let written = wrote.started + Duration::from_secs_f64(wrote.secs);
    assert!(check.calls[0].started >= written, "the check read while the backup wrote");
}

#[test]
fn one_backup_at_a_time() {
    let sb = Sandbox::new();
    slow_profile(&sb, "A", 2, "");
    slow_profile(&sb, "B", 2, "");
    sb.common_env(&[("BACKUP_CRON", "@daily")]);
    let server: Server = sb.server(&[]);
    let mut waited = false;
    wait_for(60, "both backups", || {
        let (a, b) = (server.collection("A"), server.collection("B"));
        let activity = [&a["running"][0]["activities"][0], &b["running"][0]["activities"][0]];
        assert!(!activity.iter().all(|t| t["on"][1] == "backing up"), "two backups at once: {a} {b}");
        waited |= activity.iter().any(|t| t["doing"]["lock"] == "cpu" && t["doing"]["by"][0]["trigger"] == "schedule");
        a["snapshot"]["last_ok"].is_string() && b["snapshot"]["last_ok"].is_string()
    });
    assert!(waited, "one backup should have waited for the other");
}

/// A copy that is due while a command's backup holds its lock waits silently. A request would be
/// skipped, and its record would wake the scheduler again.
#[test]
fn a_due_copy_waits_for_a_backup_elsewhere() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 5, "[copy]\ntargets = ['Slow-remote']\n");
    sb.target("Slow-remote", "Slow", "");
    let mut cli = sb.spawn(&["backup", "--collection", "Slow"], &[]);
    wait_for(10, "the command's backup", || {
        sb.store().unfinished(Some("Slow")).iter().any(|r| !r.holds().using.is_empty())
    });
    // A snapshot the target lacks: a copy is due.
    sb.store().apply(&[Event::Pending { collection: "Slow".into(), targets: vec!["Slow-remote".into()] }]).unwrap();
    let server = sb.server(&[]);
    thread::sleep(Duration::from_secs(2));
    let output = server.output();
    assert!(!server.logged(&json!({"event": {"kind": "running-already"}})), "{output}");
    assert!(!server.logged(&json!({"event": {"kind": "requested"}})), "{output}");
    _ = cli.wait();
}

/// What a command's run holds shows in the server, like the run itself.
#[test]
fn a_command_s_locks_show_in_the_server() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 10, "");
    let server = sb.server(&[]);
    let mut cli = sb.spawn(&["backup", "--collection", "Slow"], &[]);
    let runs = || card(&server.get("/collections/Slow").body, "runs");
    wait_for(20, "its locks", || runs().contains(r#"<span class="tag">cpu</span>"#));
    cli.kill().unwrap();
    _ = cli.wait();
    wait_for(10, "them to go with it", || !runs().contains(r#"<span class="tag">cpu</span>"#));
}

/// A queued run says what it waits for, as its own request names it: a copy waits for the backup, which copies too.
#[test]
fn a_queued_copy_says_it_waits_for_the_backup() {
    use std::io::BufRead;
    let sb = Sandbox::new();
    // A backup that lasts until the test is done with it.
    let done = sb.path("done");
    stdin_profile(&sb, "Slow", &cmd::until_exists(&done), "[copy]\ntargets = ['Slow-remote']\n");
    sb.target("Slow-remote", "Slow", "");
    // Created first: a copy refuses a primary that its first backup has yet to create.
    assert!(sb.rustic(&["-P", "Slow", "init"]).status.success());
    let server = sb.server(&[]);
    server.post_form("/collections/Slow/runs", "kind=backup");
    wait_for(20, "the backup", || server.collection("Slow")["running"][0]["activities"][0]["on"][0] == "Slow");
    let mut copy = sb.spawn(&["copy", "--collection", "Slow"], &[]);
    // The copy says when it starts to wait.
    let (said, waits) = std::sync::mpsc::channel();
    let out = std::io::BufReader::new(copy.stdout.take().unwrap());
    thread::spawn(move || {
        let mut lines = Vec::new();
        for line in out.lines().map_while(Result::ok) {
            if line.contains(r#""kind":"waiting-for""#) {
                _ = said.send(Ok(()));
                return;
            }
            lines.push(line);
        }
        _ = said.send(Err(lines.join("\n")));
    });
    let waited = waits.recv_timeout(Duration::from_secs(60)).expect("the copy says what it does");
    assert!(waited.is_ok(), "the copy did not wait for the backup: {waited:?}");
    let runs = card(&server.get("/collections/Slow").body, "runs");
    fs::write(&done, "").unwrap();
    copy.kill().unwrap();
    copy.wait().unwrap();
    assert!(runs.contains("(copy to every target, running)"), "{runs}");
}

/// A command waiting for the server's run shows there as queued, until it ends.
#[test]
fn a_waiting_command_shows_as_queued() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 20, "");
    let server = sb.server(&[]);
    server.post_form("/collections/Slow/runs", "kind=backup");
    wait_for(20, "the backup", || server.collection("Slow")["running"][0]["activities"][0]["on"][0] == "Slow");
    let mut cli = sb.spawn(&["backup", "--collection", "Slow"], &[]);
    let runs = || card(&server.get("/collections/Slow").body, "runs");
    // A killed command leaves its record, but no longer waits.
    let queued = || {
        let store = sb.store();
        store.unfinished(Some("Slow")).iter().any(|r| r.status == Status::Queued && store.alive(r))
    };
    wait_for(10, "the command to wait", queued);
    assert!(runs().contains(r#"<span class="tag">cli</span>"#), "{}", runs());
    assert!(runs().contains("(backup collection, running)"), "{}", runs());
    // Killed: its record stays, but nothing waits behind it any more.
    cli.kill().unwrap();
    let out = cli.wait_with_output().unwrap();
    let said = String::from_utf8_lossy(&out.stdout) + String::from_utf8_lossy(&out.stderr);
    let behind_web = json!({"event": {"kind": "waiting-for"}});
    assert!(
        json_has(&said, &behind_web) && json_has(&said, &json!({"trigger": "web", "locks": ["backup collection"]})),
        "{said}"
    );
    wait_for(10, "it to go", || !queued());
    // Cancelled from the web UI: it stops waiting at once, not when the backup ends.
    let cli = sb.spawn(&["backup", "--collection", "Slow"], &[]);
    let stop = || {
        let row = runs().split("<tr").find(|r| r.contains(r#"<span class="tag">cli</span>"#))?.to_owned();
        Some(row.split(r#"hx-delete=""#).nth(1)?.split('"').next()?.to_owned())
    };
    wait_for(10, "the command to wait", || stop().is_some());
    let cancelled = Instant::now();
    assert_eq!(server.delete(&stop().unwrap()).code, 200);
    let out = cli.wait_with_output().unwrap();
    assert!(cancelled.elapsed() < Duration::from_secs(3), "{:?}", cancelled.elapsed());
    assert!(json_has(&String::from_utf8_lossy(&out.stdout), &json!({"event": {"kind": "dequeued"}})), "{out:?}");
    assert_eq!(server.collection("Slow")["running"][0]["activities"][0]["on"][0], "Slow");
}

/// Requested while another process runs the collection: it waits, then runs.
#[test]
fn a_web_request_waits_for_a_command_line_run() {
    let (sb, server, mut cli) = backing_up_elsewhere();
    // The same request runs after it.
    assert_eq!(server.post_form("/collections/Slow/runs", "kind=backup").code, 201);
    assert!(cli.wait().unwrap().success());
    wait_for(30, "its own backup", || sb.snapshots("Slow").len() == 2);
    assert!(!server.logged(&json!({"event": {"kind": "running-already"}})), "{}", server.output());
}

/// Of the commands waiting behind the server's backup, only the one stopped stops. The record of one whose process
/// is gone cannot be stopped: nothing waits there.
#[test]
fn only_the_waiting_command_stopped_stops() {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 30, "[copy]\ntargets = ['Slow-remote']\n");
    sb.target("Slow-remote", "Slow", "");
    // Created first: a copy refuses a primary that its first backup has yet to create.
    assert!(sb.rustic(&["-P", "Slow", "init"]).status.success());
    let server = sb.server(&[]);
    server.post_form("/collections/Slow/runs", "kind=backup");
    wait_for(20, "the backup", || server.collection("Slow")["running"][0]["activities"][0]["on"][0] == "Slow");
    let (backup, mut copy) =
        (sb.spawn(&["backup", "--collection", "Slow"], &[]), sb.spawn(&["copy", "--collection", "Slow"], &[]));
    let queued = |kind: &str| {
        let store = sb.store();
        let waiting =
            store.unfinished(Some("Slow")).into_iter().filter(|r| r.status == Status::Queued && store.alive(r));
        waiting.filter(|r| serde_json::json!(r.kind) == kind).map(|r| r.id).next()
    };
    wait_for(10, "both to wait", || queued("backup").is_some() && queued("copy").is_some());
    let id = queued("backup").unwrap();
    assert_eq!(server.delete(&format!("/collections/Slow/runs/{id}")).code, 200);
    assert!(json_has(
        &String::from_utf8_lossy(&backup.wait_with_output().unwrap().stdout),
        &json!({"event": {"kind": "dequeued"}})
    ));
    let left = queued("copy").expect("the copy still waits");
    copy.kill().unwrap();
    _ = copy.wait();
    assert_eq!(server.delete(&format!("/collections/Slow/runs/{left}")).code, 422, "its process is gone");
}
