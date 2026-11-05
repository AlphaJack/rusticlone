//! Runs started from the web UI, their pages, and the notifications they send.

use super::*;

#[test]
fn a_failed_backup_from_the_web_notifies() {
    use std::io::{BufRead, BufReader};

    let receiver = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    receiver.set_nonblocking(true).unwrap();
    let url = format!("json://{}", receiver.local_addr().unwrap());
    let sb = docs_with_remote(&global_env(&[("NOTIFY_URLS", &url)]));
    sb.run(&["backup", "--all"]).assert_exit(0);
    let server = sb.server(&[]);
    sb.remove("source/docs");
    server.post_form("/collections/Docs/runs", "kind=backup");
    let mut connection = None;
    wait_for(30, "the notification request", || {
        connection = match receiver.accept() {
            Ok((socket, _)) => Some(socket),
            Err(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::WouldBlock, "{e}");
                None
            }
        };
        connection.is_some()
    });
    let mut socket = connection.unwrap();
    // macOS hands the accepted socket the listener's non-blocking mode, in which the timeout does not wait.
    socket.set_nonblocking(false).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    let mut reader = BufReader::new(&mut socket);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert_eq!(line, "POST / HTTP/1.1\r\n");
    let mut length = None;
    loop {
        line.clear();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if line == "\r\n" {
            break;
        }
        if let Some((key, value)) = line.split_once(':')
            && key.eq_ignore_ascii_case("content-length")
        {
            length = Some(value.trim().parse::<usize>().unwrap());
        }
    }
    let mut body = vec![0; length.unwrap()];
    reader.read_exact(&mut body).unwrap();
    let message: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(message["type"], "failure");
    assert!(message["title"].as_str().unwrap().ends_with(": Docs is now failing"), "{message}");
    // The missing source.
    let source = format!("{0}source{0}", std::path::MAIN_SEPARATOR);
    assert!(message["message"].as_str().unwrap().contains(&source), "{message}");
    socket.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
}

#[test]
fn repositories_are_maintained_one_by_one() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-remote", "Docs-usb"], "");
    sb.target("Docs-remote", "Docs", "");
    sb.target("Docs-usb", "usb", "");
    sb.write("docs/a.txt", "version 1");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.write("docs/a.txt", "version 2");
    assert!(sb.rustic(&["-P", "Docs", "backup"]).status.success());
    let server = sb.server(&[]);

    // Copy to one target only.
    server.post_form("/collections/Docs/repos/Docs-usb/runs", "kind=copy");
    wait_for(30, "the copy", || sb.snapshots("Docs-usb").len() == 2);
    assert_eq!(sb.snapshots("Docs-remote").len(), 1);

    // Check one repository.
    server.post_form("/collections/Docs/repos/Docs-remote/runs", "kind=check");
    wait_for(30, "the check", || server.collection("Docs")["targets"][0]["check"]["last_ok"].is_string());
    let profile = server.collection("Docs");
    assert!(profile["primary"]["check"].is_null() && profile["targets"][1]["check"].is_null(), "{profile}");
}

#[test]
fn a_check_of_a_collection_is_one_run_with_a_step_per_repository() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.run(&["check", "--all"]).assert_exit(0);
    let check = sb.store().runs("Docs", 1).remove(0);
    let steps: Vec<String> = check.steps.iter().map(|s| s.label.clone()).collect();
    assert_eq!(
        (check.label.as_str(), steps),
        ("check collection", vec!["check Docs 1/10".into(), "check Docs-remote 1/10".into()])
    );
}

#[test]
fn a_backup_unknown_until_one_is_made() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &[], "[backup]\nskip-if-unchanged = true\n");
    sb.write("docs/a.txt", "version 1");
    // An old snapshot, made with rustic.
    assert!(sb.rustic(&["-P", "Docs", "backup", "--init", "--time", "2020-01-01 00:00:00+0000"]).status.success());
    // Unchanged sources make no snapshot: the last backup is unknown, not stale.
    let status = sb.run(&["collections", "--refresh"]);
    status.assert_exit(0).assert_no_result("stale", json!({}));
    assert!(status.json()["collections"][0]["last_backup"].is_null(), "{}", status.stdout);
    let server = sb.server(&[]);
    // Nothing changed, but the backup counts as done now.
    sb.run(&["backup", "--all"]).assert_exit(0).assert_result("backed-up", json!({"snapshots": []}));
    let repos = card(&server.get("/collections/Docs").body, "repos");
    assert!(repos.contains("ago</time></span></span></td>"), "{repos}");
    // And there is nowhere to copy to: no Copy button.
    assert!(!card(&server.get("/collections/Docs").body, "repos").contains(r##"<use href="#copy"/>"##));
    assert_eq!(server.post_form("/collections/Docs/runs", "kind=copy").code, 422);
    assert_eq!(server.post_form("/collections/Docs/repos/Docs/runs", "kind=copy").code, 422);
}

#[test]
fn a_failure_can_be_dismissed() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["missing"]], &[], "");
    let missing = format!("{}missing", std::path::MAIN_SEPARATOR);
    sb.run(&["backup", "--all"]).assert_exit(1).assert_result("failed", json!({})).assert_output(&missing);
    let server = sb.server(&[]);
    assert!(server.get("/collections/Docs").body.contains("/failures/snapshot%3ADocs"));
    assert_eq!(server.delete("/collections/Docs/failures/check%3ADocs").code, 422, "not shown, not dismissed");
    assert_eq!(server.delete("/collections/Docs/failures/snapshot%3ADocs").code, 200);
    // A copy step names its target after a slash.
    assert_eq!(server.delete("/collections/Docs/failures/copy%3ADocs%2FDocs-usb").code, 422);
    let issues = server.collection("Docs")["issues"].to_string();
    assert!(!json_has(&issues, &json!({"kind": "failed"})), "{issues}");
}

/// Each check takes its subset when queued: the second reads the next subset, not the same one.
#[test]
fn checks_queued_in_a_row_read_different_subsets() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let server = sb.server(&[]);
    let (first, second) = (
        server.post_form("/collections/Docs/repos/Docs/runs", "kind=check").body,
        server.post_form("/collections/Docs/repos/Docs/runs", "kind=check").body,
    );
    assert!(flash(&first, "success") && flash(&second, "success"), "{first}\n{second}");
    let read = |n: u32| json!({"kind": "verified", "reading": {"subset": {"n": n, "of": 10}}});
    wait_for(60, "both checks", || {
        let results: Vec<_> = sb
            .store()
            .runs("Docs", 2)
            .iter()
            .flat_map(|r| r.all_results().map(|l| json!(l)).collect::<Vec<_>>())
            .collect();
        [1, 2].iter().all(|n| results.iter().any(|r| crate::common::has(r, &read(*n))))
    });
}

#[test]
fn every_job_is_backed_up_at_once() {
    let sb = docs_with_remote("");
    let server = sb.server(&[]);
    let started = server.post_form("/runs", "kind=backup");
    assert!(started.code == 201 && flash(&started.body, "success"), "{}", started.body);
    wait_for(60, "the backup", || !sb.snapshots("Docs").is_empty());
    assert_eq!(server.post_form("/runs", "kind=unknown").code, 422);
    assert!(flash(&server.post("/refresh").body, "success"));
}

#[test]
fn unfinished_requests_survive_a_restart() {
    let sb = Sandbox::new();
    crate::common::slow_profile(&sb, "Slow", 30, "");
    let mut server = sb.server(&[]);
    server.post_form("/collections/Slow/runs", "kind=backup");
    wait_for(20, "the backup", || server.collection("Slow")["running"][0]["activities"][0]["on"][0] == "Slow");
    let test = server.post_form("/collections/Slow/runs", "kind=test-restore");
    assert!(flash(&test.body, "success"), "{}", test.body);
    // Stopped (`systemctl stop`): unfinished requests are kept, the interrupted backup and the test
    // restore waiting for it.
    assert!(crate::common::signal(server.pid(), "TERM"));
    wait_for(20, "the server to stop", || !server.running());
    let runs = sb.store().runs("Slow", usize::MAX);
    let seen: Vec<String> = runs.iter().map(|r| format!("{} {}", json!(r.kind), json!(r.status))).collect();
    assert_eq!(sb.queued("Slow"), 2, "the backup and the test restore are both queued again: {seen:?}");
    drop(server);
    // Restarted: the waiting requests run.
    let _server = sb.server(&[]);
    let tested =
        || sb.store().runs("Slow", usize::MAX).iter().any(|r| json!(r.kind) == "test-restore" && r.finished.is_some());
    wait_for(60, "the test restore", tested);
    wait_for(90, "both to end", || sb.queued("Slow") == 0);
}

#[test]
fn notifications_are_sent_on_request() {
    let (url, received) = receiver();
    let sb = docs_with_remote(&global_env(&[("NOTIFY_URLS", &url)]));
    let server = sb.server(&[]);
    let sent = server.post("/collections/Docs/notifications");
    assert!(sent.code == 200 && flash(&sent.body, "success"), "{}", sent.body);
    received.recv_timeout(Duration::from_secs(15)).unwrap();
    assert_eq!(server.post("/collections/Unknown/notifications").code, 404);
    // The setup form's URLs, before they are saved.
    let tested = server.post_form("/setup/notifications", &format!("notify={url}"));
    assert!(tested.code == 200 && flash(&tested.body, "success"), "{}", tested.body);
    received.recv_timeout(Duration::from_secs(15)).unwrap();
    assert_eq!(server.post_form("/setup/notifications", "notify=invalid%3A%2F%2Ftarget").code, 422);
}

#[test]
fn an_undelivered_notification_is_refused() {
    // Nothing listens on port 9 (discard).
    let sb = docs_with_remote(&global_env(&[("NOTIFY_URLS", "json://127.0.0.1:9")]));
    let server = sb.server(&[]);
    assert_eq!(server.post("/collections/Docs/notifications").code, 422);
}

#[test]
fn stopping_what_does_not_run_is_refused() {
    let sb = docs_with_remote("");
    let server = sb.server(&[]);
    assert_eq!(server.delete("/collections/Docs/runs/1").code, 422);
    assert_eq!(server.delete("/collections/Docs/runs/1/steps/0").code, 422);
    // Every run of the collection: there is none to stop, and nothing to refuse.
    assert_eq!(server.delete("/collections/Docs/runs").code, 200);
}

/// A run's page goes on showing its log from where the page left it, so that no line is shown twice or lost.
#[test]
fn a_run_s_page_follows_its_log_from_where_it_left_it() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let id = sb.store().runs("Docs", 1)[0].id;
    let server = sb.server(&[]);
    let page = server.get(&format!("/collections/Docs/runs/{id}"));
    assert_eq!(page.code, 200);
    assert!(page.body.contains(&format!("&amp;log={id}&amp;at=")), "{}", page.body);
}
