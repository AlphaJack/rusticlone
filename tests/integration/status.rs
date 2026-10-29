use serde_json::json;

use crate::common::{Sandbox, docs_with_remote};

#[test]
fn refresh_measures_lag_the_state_does_not_know_about() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.write("docs/a.txt", "version 2");
    assert!(sb.rustic(&["-P", "Docs", "backup"]).status.success());
    sb.run(&["collections"]).assert_exit(0).assert_json(&json!({"name": "Docs-remote", "status": {"kind": "in-sync"}}));
    // Behind, by less than the schedule allows.
    sb.run(&["collections", "--refresh"])
        .assert_exit(0)
        .assert_json(&json!({"name": "Docs-remote", "status": {"kind": "behind"}}));
}

#[test]
fn a_repository_not_backed_up_for_long_is_stale() {
    let sb = docs_with_remote("");
    assert!(sb.rustic(&["-P", "Docs", "init"]).status.success());
    assert!(sb.rustic(&["-P", "Docs", "backup", "--time", "2020-01-01 00:00:00+0000"]).status.success());
    sb.run(&["collections", "--refresh"])
        .assert_exit(2)
        .assert_json(&json!({"name": "Docs", "status": {"kind": "stale"}}))
        .assert_result("stale", json!({"repo": "Docs"}));
}

#[test]
fn refresh_notices_a_target_that_vanished() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.remove("remote/Docs");
    // A warning, as when it cannot be reached: the backups are still in the primary.
    sb.run(&["collections", "--refresh"])
        .assert_exit(2)
        .assert_json(&json!({"name": "Docs-remote", "status": {"kind": "missing", "primary": false}}))
        .assert_result("missing", json!({"repo": "Docs-remote"}));
    sb.run(&["recreate", "--repo", "Docs-remote"]).assert_exit(0).assert_result("copied", json!({"snapshots": 1}));
}

#[test]
fn a_target_seen_with_snapshots_is_missing_once_gone() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    // The state is lost with the record of its copies, and its snapshots are listed again.
    sb.remove("store");
    sb.run(&["collections", "--refresh"]).assert_exit(0);
    sb.remove("remote/Docs");
    sb.run(&["collections"]).assert_exit(2).assert_result("missing", json!({"repo": "Docs-remote"}));
}

/// Rustic reports missing sources: they are not checked beforehand.
#[test]
fn a_missing_source_fails_the_backup() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs", "mail"]], &[], "");
    sb.write("docs/a.txt", "a");
    sb.run(&["backup", "--all"]).assert_exit(1).assert_result("failed", json!({}));
    sb.run(&["collections"]).assert_exit(1).assert_result("failed", json!({}));
}

#[test]
fn progress_is_reported_while_rustic_works() {
    let sb = docs_with_remote("");
    // Enough data for rustic to report progress a few times.
    sb.write("docs/big.bin", &"0123456789abcdef".repeat(1 << 20));
    let run = sb.run(&["backup", "--all"]);
    // Rustic's progress JSON is consumed, and the snapshot check still finds the new snapshot.
    run.assert_exit(0).assert_result("backed-up", json!({})).assert_result("copied", json!({"snapshots": 1}));
    let progress: Vec<serde_json::Value> = run
        .stdout
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .filter(|e| e["target"] == "progress" && e.get("percent_done").is_some())
        .collect();
    // The backup's and the copy's.
    let doing: std::collections::BTreeSet<&str> = progress.iter().filter_map(|e| e["doing"].as_str()).collect();
    assert!(doing.len() >= 2, "{}", run.stdout);
    assert!(progress.iter().all(|e| {
        let pct = e["percent_done"].as_f64().unwrap();
        (0.0..=1.0).contains(&pct) && e["collection"] == "Docs"
    }));
}

#[test]
fn profiles_with_json_progress_work() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-remote"], "[forget]\nkeep-last = 1\n");
    sb.target("Docs-remote", "Docs", "");
    // The collection itself enables JSON progress for every command.
    let toml = std::fs::read_to_string(sb.profile_path("Docs")).unwrap();
    sb.profile("Docs", &toml.replace("[global]\n", "[global]\njson-progress = true\n"));
    sb.write("docs/a.txt", "version 1");
    sb.run(&["backup", "--all"])
        .assert_exit(0)
        .assert_result("copied", json!({"target": "Docs-remote", "snapshots": 1}));
    sb.write("docs/a.txt", "version 2");
    sb.run(&["backup", "--all"]).assert_exit(0).assert_result("forgot", json!({"repo": "Docs", "snapshots": 1}));
    sb.run(&["check", "--all"]).assert_exit(0);
    sb.remove("source");
    sb.run(&["restore", "--all"]).assert_exit(0);
    assert_eq!(std::fs::read_to_string(sb.path("source/docs/a.txt")).unwrap(), "version 2");
}

/// Status changes are deduplicated; delivery failures are retried.
#[test]
fn status_changes_are_notified() {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::Duration,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("json://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    let receiver = thread::spawn(move || {
        for status in [204, 204, 500] {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
            let mut reader = BufReader::new(&mut socket);
            let mut length = 0;
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                if let Some((key, value)) = line.split_once(':')
                    && key.eq_ignore_ascii_case("content-length")
                {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let message: serde_json::Value = serde_json::from_slice(&body).unwrap();
            write!(socket, "HTTP/1.1 {status} Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            tx.send(message).unwrap();
        }
    });
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &[], &crate::common::global_env(&[("NOTIFY_URLS", &url)]));
    let receive = || rx.recv_timeout(Duration::from_secs(15)).unwrap();
    sb.run(&["backup", "--all"]).assert_exit(1);
    let message = receive();
    assert_eq!(message["type"], "failure");
    assert!(message["title"].as_str().unwrap().ends_with(": Docs is now failing"), "{message}");
    // What is missing.
    assert!(message["message"].as_str().unwrap().contains(&sb.path("source/docs").display().to_string()));
    sb.run(&["backup", "--all"]).assert_exit(1);
    assert!(rx.try_recv().is_err());
    sb.write("docs/a.txt", "a");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let message = receive();
    assert_eq!(message["type"], "success");
    assert!(message["title"].as_str().unwrap().ends_with(": Docs is now healthy"), "{message}");
    // How long it was failing, from when that was notified.
    assert!(message["message"].as_str().unwrap().starts_with("🟢 Was failing for "), "{message}");
    sb.remove("source/docs");
    sb.run(&["backup", "--all"]).assert_exit(1).assert_result("undelivered", json!({}));
    assert_eq!(receive()["type"], "failure");
    // Sent again after a backoff, not at once.
    sb.run(&["backup", "--all"]).assert_exit(1);
    assert!(rx.try_recv().is_err());
    receiver.join().unwrap();
}

/// Listed without a snapshot: never backed up. Only a listing can show this.
#[test]
fn an_empty_repository_was_never_backed_up() {
    let sb = docs_with_remote("");
    assert!(sb.rustic(&["-P", "Docs", "init"]).status.success());
    sb.run(&["collections"]).assert_result("never-backed-up", json!({}));
}

/// A new state knows nothing: the repositories are listed once, instead of showing "never backed up".
#[test]
fn a_fresh_state_is_filled_from_the_repositories() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.remove("store");
    let status = sb.run(&["collections"]);
    status
        .assert_exit(0)
        .assert_no_result("never-backed-up", json!({}))
        .assert_json(&json!({"name": "Docs-remote", "status": {"kind": "in-sync"}}));
    // Listed only once, while the state knew nothing.
    let calls = |out: &str| out.matches("rustic snapshots").count();
    assert!(calls(&status.output) >= 2, "{}", status.output);
    assert_eq!(calls(&sb.run(&["collections"]).output), 0);
}

/// Where unchanged sources make no snapshot, the newest snapshot dates the last change, not the last
/// backup.
#[test]
fn unchanged_sources_leaving_no_snapshot_are_not_stale() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &[], "[forget]\nkeep-last = 10\ndelete-unchanged = true\n");
    sb.write("docs/a.txt", "a");
    assert!(sb.rustic(&["-P", "Docs", "backup", "--init", "--time", "2020-01-01 00:00:00+0000"]).status.success());
    // Its last backup is unknown, so it is not stale.
    let status = sb.run(&["collections"]);
    status.assert_exit(0).assert_no_result("stale", json!({}));
    assert!(status.json()["collections"][0]["last_backup"].is_null(), "{}", status.stdout);
    // A backup of unchanged sources: its snapshot is removed, but its record stays.
    sb.run(&["backup", "--all"]).assert_exit(0).assert_result("forgot", json!({"repo": "Docs", "snapshots": 1}));
    assert_eq!(sb.snapshots("Docs").len(), 1);
    let status = sb.run(&["collections"]);
    status.assert_exit(0);
    assert!(status.json()["collections"][0]["last_backup"].is_string(), "{}", status.stdout);
}

/// A target in sync with a stale primary is stale too, and reported once.
#[test]
fn stale_job_is_notified_once() {
    let sb = docs_with_remote("");
    assert!(sb.rustic(&["-P", "Docs", "backup", "--init", "--time", "2020-01-01 00:00:00+0000"]).status.success());
    assert!(sb.rustic(&["-P", "Docs", "copy", "--target", "Docs-remote", "--init"]).status.success());
    let status = sb.run(&["collections"]);
    status
        .assert_exit(2)
        .assert_result("stale", json!({"repo": "Docs"}))
        .assert_no_result("stale", json!({"repo": "Docs-remote"}));
}

/// The tests run the rustic version whose output is parsed. A newer one needs its output recorded
/// (`cargo xtask fixtures`) and `[workspace.metadata.rustic]` raised.
#[test]
fn tests_run_against_the_tested_rustic() {
    let sb = docs_with_remote("");
    sb.run(&["collections"]).assert_no_result("rustic-untested", json!({}));
}

/// Pruning on every forget is slow, not unsafe: it is reported, and runs.
#[test]
fn a_job_pruning_on_every_forget_is_warned_about() {
    let sb = docs_with_remote("[forget]\nkeep-last = 3\nprune = true\n");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.run(&["collections"]).assert_exit(2).assert_result("prunes-on-forget", json!({"repo": "Docs"}));
}

#[test]
fn a_collection_is_named_after_what_its_profiles_share() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs-usb", &[&["docs"]], &["Docs-s3"], "");
    sb.target("Docs-s3", "Docs", "");
    sb.write("docs/a.txt", "version 1");
    sb.run(&["backup", "--collection", "Docs"]).assert_exit(0).assert_result("backed-up", json!({"repo": "Docs-usb"}));
    sb.run(&["collections"]).assert_exit(0).assert_json(&json!({"title": "Docs", "primary": {"name": "Docs-usb"}}));
}

/// `notify` sends the status on request; in a dry run, it says what it would send.
#[test]
fn notify_says_what_it_sends() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &[], &crate::common::global_env(&[("NOTIFY_URLS", "json://127.0.0.1:9")]));
    sb.write("docs/a.txt", "a");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.run(&["notify", "--all", "--dry-run"]).assert_exit(0).assert_result("would-notify", json!({}));
    // Nothing listens on port 9.
    sb.run(&["notify", "--all"]).assert_exit(1).assert_result("undelivered", json!({}));
    sb.run(&["notify"]).assert_exit(1);
}

/// What a person reads: every integration test above reads `--json`.
#[test]
fn collections_are_shown_as_a_tree() {
    let sb = hdd_and_remote(&["Docs-remote"]);
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.profile("unused", "[repository]\nrepository = '/nowhere'\npassword = 'x'\n");
    sb.profile("broken", "[repository\n");
    let run = sb.run_text(&["collections", "--refresh"]);
    run.assert_exit(1);
    insta::assert_snapshot!(readable(&sb, &run.stdout));
}

#[test]
fn a_failing_target_is_shown_with_its_issues() {
    let sb = hdd_and_remote(&["Docs-usb", "Docs-remote"]);
    std::fs::write(sb.path("remote/unplugged"), "").unwrap();
    sb.target("Docs-usb", "unplugged/Docs", "");
    sb.run(&["backup", "--all"]).assert_exit(1);
    let run = sb.run_text(&["collections"]);
    run.assert_exit(2);
    insta::assert_snapshot!(readable(&sb, &run.stdout));
}

/// The repositories are created by the first backup: until then, a refresh finds none, and is not failing.
#[test]
fn a_refresh_before_the_first_backup_is_not_a_failure() {
    let sb = hdd_and_remote(&["Docs-remote"]);
    let run = sb.run_text(&["collections", "--refresh"]);
    run.assert_exit(2);
    assert!(!run.stdout.contains("unavailable") && run.stdout.contains("● never backed up"), "{}", run.stdout);
}

/// Rustic would ask for it, with no one to type it. The repository is not created, so that rustic, which
/// reads its config first, asks nothing.
#[test]
fn a_repository_without_a_password_is_an_issue() {
    let sb = Sandbox::new();
    let (repo, source) = (sb.path("local/Docs").display().to_string(), sb.path("source/docs").display().to_string());
    let docs = |password: &str| {
        format!("[repository]\nrepository = '{repo}'\n{password}\n[[backup.snapshots]]\nsources = ['{source}']\n")
    };
    sb.profile("Docs", &docs(""));
    sb.run(&["collections"]).assert_exit(1).assert_json(&json!({"kind": "no-password", "repo": "Docs"}));
    sb.profile("Docs", &docs("password = 'x'"));
    let run = sb.run(&["collections"]);
    assert!(!run.stdout.contains("no-password"), "{}", run.stdout);
}

#[test]
fn a_backup_profile_without_sources_is_invalid() {
    let sb = Sandbox::new();
    sb.profile("Docs", "[global]\nuse-profiles = ['common']\n\n[repository]\nrepository = '/nowhere'\n\n[[backup.snapshots]]\nsources = []\n");
    sb.run(&["collections"]).assert_exit(1).assert_json(&json!({"name": "Docs", "kind": "broken"}));
    let text = sb.run_text(&["collections"]).stdout;
    assert!(
        text.contains("└──┐Invalid\n   └── Docs: its [[backup.snapshots]] list no sources") && !text.contains("Unused"),
        "{text}"
    );
}

/// Profiles, but none a collection: what is missing is said, as without any profile.
#[test]
fn profiles_without_a_collection_say_what_is_missing() {
    let sb = Sandbox::new();
    let text = sb.run_text(&["collections"]).stdout;
    assert!(text.starts_with("Profile folders\n") && text.contains("No collections yet"), "{text}");
    sb.target("Docs-remote", "Docs", "");
    let text = sb.run_text(&["collections"]).stdout;
    assert!(text.contains("No collections yet") && text.contains("Other profiles"), "{text}");
}

/// What a person reads after a backup: a line of its own starts with a capital, and a result about the
/// collection's own repository does not name it twice.
#[test]
fn backup_lines_read_as_sentences() {
    let sb = Sandbox::new();
    let none = sb.run_text(&["backup", "--all"]).stdout;
    assert!(none.starts_with("No profile with backup sources"), "{none}");
    let sb = docs_with_remote("");
    let backed_up = sb.run_text(&["backup", "--all"]).stdout;
    assert!(backed_up.contains("Docs: backed up") && !backed_up.contains("Docs: Docs:"), "{backed_up}");
}

/// Profiles, none a collection: the hint, then every group of the other profiles.
#[test]
fn profiles_without_a_collection_are_shown_as_a_tree() {
    let sb = Sandbox::new();
    sb.profile("Docs", "[global]\nuse-profiles = ['common']\n\n[repository]\nrepository = '/nowhere'\n\n[[backup.snapshots]]\nsources = []\n");
    sb.target("Music", "music", "");
    sb.profile("broken", "[repository\n");
    let run = sb.run_text(&["collections"]);
    run.assert_exit(1);
    insta::assert_snapshot!(readable(&sb, &run.stdout));
}

/// The collection `Docs`, as people name it: its primary `Docs-hdd`, copied to `targets`, and `Docs-remote`.
fn hdd_and_remote(targets: &[&str]) -> Sandbox {
    let sb = Sandbox::new();
    sb.backup_profile("Docs-hdd", &[&["docs"]], targets, "");
    sb.target("Docs-remote", "Docs", "");
    sb.write("docs/a.txt", "version 1");
    sb
}

/// `collections`' text, the same on every system: the folders rustic searches differ, and so do sizes. The
/// current folder is checked to be named, last.
fn readable(sb: &Sandbox, text: &str) -> String {
    let text = (text.replace(&sb.profile_dir().display().to_string(), "[profiles]"))
        .replace(&sb.path("").display().to_string(), "[sandbox]/")
        .replace('\\', "/");
    let (folders, rest) = text.split_once("\n\n").unwrap_or_default();
    assert!(
        folders.starts_with("Profile folders\n") && folders.ends_with("└── [sandbox]/cwd (current folder)"),
        "{text}"
    );
    let lines = rest.lines().map(|l| match l.split_once("Size: ") {
        Some((before, _)) => format!("{before}Size: [size]"),
        None => l.to_owned(),
    });
    std::iter::once("Profile folders\n[folders]\n\n".to_owned()).chain(lines.map(|l| l + "\n")).collect()
}
