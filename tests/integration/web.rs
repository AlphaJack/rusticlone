//! The web UI: repository views (listing, browsing, downloading, restoring, re-creating), live pages,
//! monitoring and logins.

use std::{
    io::{Read, Write},
    thread,
    time::{Duration, Instant},
};

use rusticlone::APP_NAME;
use serde_json::json;

use crate::{
    common::{Sandbox, docs_with_remote, global_env, json_has, wait_for},
    server::card,
};

/// Whether `html` has a disabled button with this icon.
fn disabled(html: &str, icon: &str) -> bool {
    html.contains(&format!(r##"disabled><svg class="i" aria-hidden="true"><use href="#{icon}"/>"##))
}

/// Whether a flash message of this level is in `html`: `success` or `failure`.
fn flash(html: &str, level: &str) -> bool {
    html.contains(&format!(r#"class="flash {level}""#))
}

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
fn repositories_are_listed_on_demand() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let server = sb.server(&[]);
    let snapshots = sb.snapshot_ids("Docs-remote");
    let short = snapshots[0].get(..8).unwrap();
    // Opening a repository lists it.
    assert_eq!(server.get("/collections/Docs/repos/Docs-remote").code, 200);
    wait_for(30, "the listing", || server.get("/collections/Docs/repos/Docs-remote").body.contains(short));
    // Refreshing the collection lists them too: its page shows each repository's snapshots, and the source's
    // files at the last backup.
    server.post("/collections/Docs/refresh");
    wait_for(30, "the primary's listing", || {
        let repos = card(&server.get("/collections/Docs").body, "repos");
        !repos.contains("spinner") && repos.matches(r#"<td class="right num">1</td>"#).count() == 2
    });
    assert!(card(&server.get("/collections/Docs").body, "sources").contains(r#"<td class="right num">1</td>"#));
    assert_eq!(server.get("/collections/Docs/repos/Unknown").code, 404);
    // The location of each repository.
    let remote = sb.path("remote/Docs").display().to_string();
    assert!(server.get("/collections/Docs").body.contains(&format!(r#"data-copy="{remote}""#)));
    assert_eq!(server.collection("Docs")["targets"][0]["repository"], remote);
}

#[test]
fn snapshots_can_be_browsed_and_downloaded() {
    let sb = docs_with_remote("");
    sb.write("docs/sub/b.txt", "inside");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let server = sb.server(&[]);
    let id = &sb.snapshot_ids("Docs")[0];
    let docs = crate::common::in_tree(&sb.path("source/docs"));
    let base = format!("/collections/Docs/repos/Docs/snapshots/{id}");
    // A snapshot of one directory opens in that directory.
    let page = server.get(&base).body;
    assert!(page.contains("a.txt") && page.contains(">sub<"), "{page}");
    let sub = server.get(&format!("{base}?path={docs}/sub")).body;
    assert!(sub.contains("b.txt") && !sub.contains("a.txt"), "{sub}");

    let file = server.get(&format!("{base}/download?path={docs}/a.txt"));
    assert!(file.code == 200 && file.body.contains("version 1"), "{}", file.body);
    assert!(file.head.contains("filename=\"a.txt\""), "{}", file.head);
    let zip = server.get(&format!("{base}/download?path={docs}/sub&dir=true"));
    assert!(zip.body.contains("PK") && zip.head.contains("sub.zip"), "{}", zip.head);
    // Ids reach rustic as arguments, never as options.
    assert_eq!(server.get("/collections/Docs/repos/Docs/snapshots/--help").code, 400);
}

/// Listing a remote snapshot can take seconds. The rest of the UI does not wait for it, and its other
/// directories are served from the cache.
#[test]
#[expect(clippy::disallowed_methods, reason = "observe external processes over a bounded interval")]
fn a_slow_listing_holds_up_nothing_else() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let id = sb.snapshot_ids("Docs")[0].clone();
    // A rustic that reads snapshots slowly, like a slow remote: first in PATH, or on Windows a batch file that
    // rusticlone is told to run.
    let path = std::env::var_os("PATH").unwrap();
    let name = if cfg!(windows) { "rustic.exe" } else { "rustic" };
    let rustic = std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file()).unwrap();
    let dir = sb.path("slow");
    std::fs::create_dir_all(&dir).unwrap();
    #[cfg(unix)]
    let env = {
        let slow = dir.join("rustic");
        // Quoted, so that a path with spaces, as macOS's Application Support, stays one word.
        let quoted = rustic.display().to_string().replace('\'', r"'\''");
        let script = format!("#!/bin/sh\ncase \" $* \" in *\" ls \"*) sleep 2;; esac\nexec '{quoted}' \"$@\"\n");
        std::fs::write(&slow, script).unwrap();
        std::fs::set_permissions(&slow, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let path = std::env::join_paths(std::iter::once(dir).chain(std::env::split_paths(&path))).unwrap();
        ("PATH".to_owned(), path.into_string().unwrap())
    };
    #[cfg(windows)]
    let env = {
        let slow = dir.join("rustic.cmd");
        let script = format!(
            "@echo off\r\necho  %* | findstr /c:\" ls \" >nul && ping -n 3 127.0.0.1 >nul\r\n\
             \"{}\" %*\r\nexit /b %errorlevel%\r\n",
            rustic.display()
        );
        std::fs::write(&slow, script).unwrap();
        (crate::common::var("RUSTIC"), slow.display().to_string())
    };
    let server = sb.server(&[(env.0.as_str(), env.1.as_str())]);
    let browsing = thread::spawn({
        let (addr, token, id) = (server.addr.clone(), server.token.clone(), id.clone());
        move || {
            let mut stream = std::net::TcpStream::connect(addr).unwrap();
            let path = format!("/collections/Docs/repos/Docs/snapshots/{id}");
            let auth = format!("Authorization: Bearer {token}");
            write!(stream, "GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n{auth}\r\n\r\n").unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        }
    });
    thread::sleep(Duration::from_millis(500));
    let started = Instant::now();
    assert!(server.get("/health").code == 200 && server.get("/collections/Docs").code == 200);
    assert!(started.elapsed() < Duration::from_secs(2), "waited {:?}", started.elapsed());
    assert!(browsing.join().unwrap().contains("a.txt"));
    // The whole snapshot is listed on the first request.
    let started = Instant::now();
    let root = server.get(&format!("/collections/Docs/repos/Docs/snapshots/{id}?path=/")).body;
    let top = crate::common::in_tree(&sb.path("")).split('/').nth(1).unwrap().to_owned();
    assert!(root.contains(&format!(">{top}<")), "{root}");
    assert!(started.elapsed() < Duration::from_millis(500), "{:?}", started.elapsed());
}

#[test]
fn a_path_is_restored_from_a_copy_target() {
    let sb = docs_with_remote("");
    sb.write("docs/sub/b.txt", "inside");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let server = sb.server(&[]);
    let id = &sb.snapshot_ids("Docs-remote")[0];
    let (docs, to) = (sb.path("source/docs").display().to_string(), sb.path("restored"));
    let new = |query: &str| server.get(&format!("/collections/Docs/runs/new?{query}"));
    // What each form restores: a path, one snapshot, or every source.
    let path = new(&format!("kind=restore&repo=Docs-remote&snapshot={id}&path={docs}/sub")).body;
    assert!(path.contains(&format!("<code>{docs}/sub</code>")) && path.contains("<b>Docs-remote</b>"), "{path}");
    let whole = new(&format!("kind=restore&repo=Docs-remote&snapshot={id}")).body;
    assert!(whole.contains(&format!("<code>{docs}</code>")), "{whole}");
    assert!(new("kind=restore").body.contains(&format!("<code>{docs}</code>")));
    assert_eq!(new("kind=backup").code, 404);
    // In the page's dialog, only the form is returned.
    let dialog = server.get_with("/collections/Docs/runs/new?kind=restore", &["HX-Request: true"]).body;
    assert!(dialog.trim_start().starts_with("<form") && !dialog.contains("<title>"), "{dialog}");
    let form = format!("kind=restore&repo=Docs-remote&snapshot={id}&path={docs}/sub&to={}", to.display());
    assert!(flash(&server.post_form("/collections/Docs/runs", &form).body, "success"));
    wait_for(30, "the restore", || sb.store().runs("Docs", 1).first().is_some_and(|r| r.finished.is_some()));
    assert_eq!(std::fs::read_to_string(to.join("sub/b.txt")).unwrap(), "inside");
    assert_eq!(std::fs::read_dir(&to).unwrap().count(), 1, "only the path");
    let relative = server.post_form("/collections/Docs/runs", "kind=restore&to=relative");
    assert!(relative.code == 422 && flash(&relative.body, "failure"), "{}", relative.body);
}

/// The restore form restores the newest snapshots not after a time.
#[test]
fn a_restore_from_the_web_takes_a_time() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.write("docs/a.txt", "changed");
    sb.write("docs/b.txt", "new");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let server = sb.server(&[]);
    let form = server.get("/collections/Docs/runs/new?kind=restore").body;
    assert!(form.contains(r#"name="at""#), "{form}");
    // At the time of the first backup: a.txt as it was then, b.txt not in that snapshot.
    sb.remove("source/docs/b.txt");
    let first = sb.snapshots("Docs")[0].clone();
    let restore = format!("kind=restore&at={}", first.replace('+', "%2B"));
    assert!(flash(&server.post_form("/collections/Docs/runs", &restore).body, "success"));
    wait_for(30, "the restore", || sb.store().runs("Docs", 1).first().is_some_and(|r| r.finished.is_some()));
    assert_eq!(std::fs::read_to_string(sb.path("source/docs/a.txt")).unwrap(), "version 1");
    assert!(!sb.path("source/docs/b.txt").exists());
    // A dry run with choices says what it would restore where, and restores nothing.
    let dry = format!("kind=restore&dry_run=true&to={}", sb.path("elsewhere").display());
    assert!(flash(&server.post_form("/collections/Docs/runs", &dry).body, "success"));
    wait_for(30, "the dry run", || sb.store().runs("Docs", 1).first().is_some_and(|r| r.finished.is_some()));
    assert_eq!(sb.store().runs("Docs", 1)[0].label, "restore (dry-run)");
    assert!(!sb.path("elsewhere").exists());
    let wrong = server.post_form("/collections/Docs/runs", "kind=restore&at=someday");
    assert!(wrong.code == 422 && flash(&wrong.body, "failure"), "{}", wrong.body);
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
fn lost_repositories_are_neither_maintained_nor_restored_from() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.remove("local/Docs");
    sb.remove("remote/Docs");
    let server = sb.server(&[]);
    server.post_form("/collections/Docs/runs", "kind=check");
    let skipped = json!({"kind": "missing-skipped", "repo": "Docs-remote"});
    wait_for(30, "the checks", || {
        sb.store().runs("Docs", 2).iter().any(|r| r.all_results().any(|l| crate::common::has(&json!(l), &skipped)))
    });
    let page = server.get("/collections/Docs").body;
    let issues = server.collection("Docs")["issues"].to_string();
    let missing = json!({"kind": "missing", "repo": "Docs-remote"});
    assert!(!json_has(&issues, &json!({"kind": "failed"})) && json_has(&issues, &missing), "{issues}");
    // Their listings are gone with them.
    let status = server.collection("Docs").to_string();
    assert_eq!(status.matches(r#""status":{"kind":"missing","#).count(), 2, "{status}");
    assert!(!card(&page, "repos").contains(r#"<td class="right num">1</td>"#), "{page}");
    assert!(disabled(&card(&page, "sources"), "restore"));
}

#[test]
fn a_vanished_target_is_not_in_sync() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let server = sb.server(&[]);
    assert!(server.collection("Docs")["targets"][0]["lag"]["behind_since"].is_null());
    sb.remove("remote/Docs");
    server.post("/collections/Docs/repos/Docs-remote/refresh");
    // The listing records the lag before it ends, and the page shows its error once it has.
    wait_for(30, "the listing to fail", || {
        server.get("/collections/Docs/repos/Docs-remote").body.contains("notice failure")
    });
    let docs = server.collection("Docs");
    assert!(docs["targets"][0]["lag"]["behind_since"].is_string(), "{docs}");
    assert_ne!(docs["targets"][0]["status"]["kind"], "in-sync");
}

#[test]
fn sources_are_shown_per_snapshot() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"], &["mail", "notes"]], &[], "");
    let toml = std::fs::read_to_string(sb.profile_path("Docs")).unwrap();
    sb.profile("Docs", &toml.replacen("[[backup.snapshots]]\n", "[[backup.snapshots]]\nlabel = 'papers'\n", 1));
    let server = sb.server(&[]);
    let sources = card(&server.get("/collections/Docs").body, "sources");
    let (docs, notes) = (sb.path("source/docs").display().to_string(), sb.path("source/notes").display().to_string());
    assert!(sources.contains(&format!(">{docs}</code>")) && sources.contains(&format!(">{notes}</code>")), "{sources}");
}

#[test]
fn schedules_are_marked() {
    let sb = Sandbox::new();
    let env = &global_env(&[("BACKUP_CRON", "@daily"), ("CHECK_CRON", "0 3 * * *")]);
    sb.backup_profile("Daily", &[&["a"]], &[], env);
    sb.backup_profile("Manual", &[&["b"]], &[], "");
    let server = sb.server(&[]);
    let overview = server.get("/").body;
    // Lit with its expression, or not.
    let schedules: Vec<&str> = overview.split(r#"<span class="schedule"#).skip(1).collect();
    assert!(schedules.iter().any(|s| s.starts_with(" on") && s.contains("@daily")), "{overview}");
    assert!(schedules.iter().any(|s| s.starts_with(r#"" title="#)), "{overview}");
    let repos = card(&server.get("/collections/Daily").body, "repos");
    let schedules: Vec<&str> = repos.split(r#"<span class="schedule"#).skip(1).collect();
    assert!(schedules.iter().any(|s| s.starts_with(" on") && s.contains("0 3 * * *")), "{repos}");
    assert!(schedules.iter().any(|s| s.starts_with(r#"" title="#)), "{repos}");
    let (daily, manual) = (server.collection("Daily"), server.collection("Manual"));
    assert!(daily["backup_schedule"]["cron"] == "@daily" && daily["backup_schedule"]["next"].is_string(), "{daily}");
    assert!(manual["backup_schedule"].is_null(), "{manual}");
}

#[test]
fn a_source_is_test_restored_from_every_repository() {
    let sb = docs_with_remote("");
    sb.write("docs/sub/b.txt", "inside");
    sb.run(&["backup", "--all"]).assert_exit(0);
    // Changed since: one file replaced, and one file not in the snapshot.
    sb.write("docs/a.txt", "version 2");
    sb.write("docs/new.txt", "new");
    let before = sb.tree("source/docs");
    let server = sb.server(&[]);
    // The restore form with its dry run checked, from any repository.
    let form = server.get("/collections/Docs/runs/new?kind=test-restore&source=0").body;
    assert!(form.contains(r#"name="dry_run" id="restore-dry-run" value="true" checked"#), "{form}");
    assert!(form.contains(r#"<option value="Docs">"#) && form.contains(r#"<option value="Docs-remote">"#));
    // Opened as a link, it is a page of its own.
    assert!(form.contains("<title>") && form.contains(&format!(" | Docs | {APP_NAME}</title>")), "{form}");
    // Started, or queued behind the server's first round.
    for repo in ["Docs", "Docs-remote"] {
        let form = format!("kind=test-restore&repo={repo}&source=0");
        assert!(flash(&server.post_form("/collections/Docs/runs", &form).body, "success"));
    }
    wait_for(60, "both tests", || {
        let runs = sb.store().runs("Docs", 10);
        runs.iter().filter(|r| r.finished.is_some() && json!(r.kind) == "test-restore").count() == 2
    });
    // Read from each repository, and nothing written.
    let docs = sb.path("source/docs").display().to_string();
    let plan =
        json!({"kind": "test-restored", "plans": [[docs, {"create": 0, "replace": 1, "keep": 1, "additional": 1}]]});
    for run in sb.store().runs("Docs", 2) {
        let results: Vec<_> = run.all_results().map(|l| json!(l)).collect();
        assert!(crate::common::has(&results[0], &plan) && results[0]["bytes"].as_u64() > Some(0), "{results:?}");
    }
    assert_eq!(sb.tree("source/docs"), before);
    // Then a real restore: what changed since the newest snapshot is restored.
    let form = server.get("/collections/Docs/runs/new?kind=restore&source=0").body;
    assert!(form.contains(r#"<option value="">"#));
    server.post_form("/collections/Docs/runs", "kind=restore&repo=&source=0");
    wait_for(30, "the restore", || std::fs::read_to_string(sb.path("source/docs/a.txt")).unwrap() == "version 1");
    assert!(sb.path("source/docs/new.txt").is_file(), "never deleted");
}

#[test]
fn sources_no_snapshot_holds_cannot_be_test_restored() {
    let sb = docs_with_remote("");
    let server = sb.server(&[]);
    server.post_form("/collections/Docs/runs", "kind=test-restore");
    wait_for(30, "the refusal", || {
        sb.store().runs("Docs", 1).first().is_some_and(|r| r.all_results().any(|l| l.values["kind"] == "failed"))
    });
}

#[test]
fn lost_repositories_are_re_created_by_copying() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let server = sb.server(&[]);
    let repos = || card(&server.get("/collections/Docs").body, "repos");

    // A copy target is lost, then re-created by a copy.
    sb.remove("remote/Docs");
    // Missing, also after it is listed again, with rustic's reason.
    let missing = || server.collection("Docs")["targets"][0]["status"]["kind"] == "missing";
    wait_for(10, "the status", missing);
    let issues = server.collection("Docs")["issues"].to_string();
    assert!(json_has(&issues, &json!({"kind": "missing", "repo": "Docs-remote"})), "{issues}");
    server.post("/collections/Docs/repos/Docs-remote/refresh");
    wait_for(30, "the listing", || {
        server.get("/collections/Docs/repos/Docs-remote").body.contains("No repository config file found")
    });
    assert!(missing());
    server.post_form("/collections/Docs/repos/Docs-remote/runs", "kind=re-create");
    wait_for(30, "the copy", || sb.snapshots("Docs-remote").len() == 1);

    // The primary is lost too, and comes back from the target.
    sb.remove("local/Docs");
    wait_for(10, "the status", || disabled(&repos(), "copy"));
    // Copy is disabled, and Re-create is offered.
    assert!(disabled(&repos(), "copy") && repos().contains(r##"<use href="#recreate"/>"##));
    assert_eq!(server.post_form("/collections/Docs/repos/Docs/runs", "kind=copy").code, 422);
    // The form offers the target with the newest snapshots, or another one.
    let form = server.get("/collections/Docs/runs/new?kind=re-create&repo=Docs").body;
    assert!(form.contains(r#"<option value="">"#), "{form}");
    assert!(form.contains(r#"<option value="Docs-remote">Docs-remote</option>"#), "{form}");
    server.post_form("/collections/Docs/repos/Docs/runs", "kind=re-create&from=Docs-remote");
    wait_for(30, "the primary to come back", || sb.snapshots("Docs").len() == 1);
    // Copied by the backup, or by the server first: the chunker matches either way.
    sb.write("docs/a.txt", "version 3");
    sb.run(&["backup", "--all"]).assert_exit(0);
    wait_for(30, "the copy", || sb.snapshots("Docs-remote").len() == 2);
}

#[test]
#[expect(clippy::disallowed_methods, reason = "observe external processes over a bounded interval")]
fn pages_get_what_changed_pushed() {
    let sb = docs_with_remote("");
    let server = sb.server(&[]);
    // A page rendered before a change receives its current version at once.
    let mut events = server.events("v=999999&collection=Docs", &[]);
    let first = events.wait_for(10, "the page", |seen| seen.contains(r#"id="repos""#));
    assert!(first.contains("text/event-stream") && first.contains("\nid: "), "{first}");
    // Then on each change it shows, also from another process: a backup.
    sb.run(&["backup", "--all"]).assert_exit(0);
    events.wait_for(10, "the backup", |seen| seen.contains(r#"<td class="right num">1</td>"#));
    // An up-to-date page waits for the next change, once the first listings are done.
    wait_for(30, "the first listings", || sb.store().inventory("Docs", "Docs-remote").is_some());
    thread::sleep(Duration::from_millis(500));
    let page = server.get("/collections/Docs").body;
    let query = page.split("/events?").nth(1).unwrap().split('"').next().unwrap().replace("&amp;", "&");
    let mut current = server.events(&query, &[]);
    let sent = current.read_for(1);
    // Something is sent at once anyway, so that browsers open the stream.
    assert!(sent.contains("\r\n: live\n") && !sent.contains("data:"), "{sent:?}");
    // A page reconnecting after missed changes receives the page at once.
    let mut again = server.events("collection=Docs", &["Last-Event-ID: 0"]);
    again.wait_for(10, "the page", |seen| seen.contains(r#"id="repos""#));
}

#[test]
fn web_failure_shows_on_every_page() {
    let sb = docs_with_remote("");
    let server = sb.server(&[]);
    sb.remove("source/docs");
    // The overview is open, not the collection's page.
    let mut events = server.events("v=999999", &[]);
    events.wait_for(10, "the page", |seen| seen.contains("<h1>"));
    server.post_form("/collections/Docs/runs", "kind=backup");
    events.wait_for(60, "the failure", |seen| seen.contains("event: flash") && seen.contains("flash failure"));
}

#[test]
fn progress_is_pushed() {
    let sb = docs_with_remote("");
    // Many files, which take rustic seconds rather than one big one that it deduplicates at once: it reports
    // progress once a second.
    for i in 0..20_000 {
        sb.write(&format!("docs/many/{}/{i}.txt", i % 100), &i.to_string());
    }
    let server = sb.server(&[]);
    let mut events = server.events("v=999999&collection=Docs", &[]);
    events.wait_for(10, "the page", |seen| seen.contains("<h1>"));
    server.post_form("/collections/Docs/runs", "kind=backup");
    events.wait_for(30, "a progress bar", |seen| seen.contains(r#"<progress max="100""#));
}

#[test]
fn edited_profiles_are_picked_up_at_once() {
    let sb = docs_with_remote("");
    let server = sb.server(&[]);
    sb.backup_profile("Mail", &[&["mail"]], &[], "");
    let listed = |name: &str| server.get("/status").body.contains(&format!("\"{name}\""));
    wait_for(10, "the new profile", || listed("Mail"));
    std::fs::remove_file(sb.profile_path("Mail")).unwrap();
    wait_for(10, "the profile to go", || !listed("Mail"));
    // Renamed to a name that is no profile's, which macOS reports as that file's creation only.
    let (path, old) = (sb.profile_path("Docs"), sb.profile_dir().join("Docs.toml.old"));
    std::fs::rename(&path, &old).unwrap();
    wait_for(10, "the renamed profile to go", || !listed("Docs"));
    std::fs::rename(&old, &path).unwrap();
    wait_for(10, "the profile to be back", || listed("Docs"));
    // Also on request (`systemctl reload`), which Windows has no signal for.
    #[cfg(unix)]
    {
        let reloads = || server.output().matches(r#""kind":"reloaded""#).count();
        let before = reloads();
        assert!(crate::common::signal(server.pid(), "HUP"));
        wait_for(10, "the reload", || reloads() > before);
    }
}

#[test]
fn listings_are_kept_across_restarts() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let server = sb.server(&[]);
    server.get("/collections/Docs/repos/Docs-remote");
    wait_for(30, "the listing", || sb.store().inventory("Docs", "Docs-remote").is_some_and(|i| i.result.is_ok()));
    drop(server);
    let server = sb.server(&[]);
    assert!(card(&server.get("/collections/Docs").body, "repos").contains(r#"<td class="right num">1</td>"#));
    assert!(sb.store().inventory("Docs", "Docs-remote").is_some());
}

#[test]
fn runs_keep_the_listings_current() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let server = sb.server(&[]);
    let repos = || card(&server.get("/collections/Docs").body, "repos");
    let counts = |n: &str| repos().matches(&format!(r#"<td class="right num">{n}</td>"#)).count();
    let stale = || repos().matches(r#"class="icon-btn small stale""#).count();
    // Listed and measured by the backup and the copy: the page made no listing of its own.
    assert_eq!(counts("1"), 2);
    assert!(!repos().contains(r#"nowrap">unknown"#) && stale() == 0, "{}", repos());
    sb.write("docs/a.txt", "version 2");
    server.post_form("/collections/Docs/runs", "kind=backup");
    // The newest run is the web UI's, and it ended.
    wait_for(60, "the backup", || {
        let page = server.get("/collections/Docs").body;
        let newest = card(&page, "runs").split("<tr").nth(2).unwrap_or_default().to_owned();
        let copied = sb.store().runs("Docs", 1)[0].all_results().any(|l| l.values["kind"] == "copied");
        newest.contains(">web<") && !newest.contains(r#"class="spinner""#) && copied
    });
    assert!(counts("2") == 2 && stale() == 0 && !repos().contains("spinner"), "{}", repos());
    // A snapshot made by rustic directly, seen by a listing: the size is from before it.
    sb.write("docs/a.txt", "version 3");
    assert!(sb.rustic(&["-P", "Docs", "backup"]).status.success());
    sb.run(&["copy", "--all"]).assert_exit(0);
    assert_eq!(stale(), 1, "{}", repos());
    // Until it is measured again.
    server.post("/collections/Docs/repos/Docs/refresh");
    wait_for(30, "the size", || stale() == 0);
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
    // And there is nowhere to copy to.
    assert!(disabled(&card(&server.get("/collections/Docs").body, "repos"), "copy"));
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

#[test]
fn actions_from_other_sites_are_refused() {
    let sb = docs_with_remote("");
    let server = sb.server(&[]);
    let auth = format!("Authorization: Bearer {}", server.token);
    // A proxy adds the token to every browser request, including cross-site ones.
    let other_site = server.request("POST", "/collections/Docs/runs", &[&auth, "Sec-Fetch-Site: cross-site"], "");
    assert_eq!(other_site.code, 403);
    let old_browser = server.request("POST", "/collections/Docs/runs", &[&auth, "Origin: https://evil.example"], "");
    assert_eq!(old_browser.code, 403);
    // Scripts send neither header.
    assert_eq!(server.request("POST", "/collections/Docs/repos/Docs/refresh", &[&auth], "").code, 200);
    // Monitors need no login, and learn only that the server runs.
    let health = server.request("GET", "/health", &[], "");
    assert!(health.code == 200 && health.body == "ok\n", "{}", health.body);
    let head = health.head.to_lowercase();
    assert!(head.contains("content-security-policy: default-src 'self';") && !head.contains("unsafe-inline"), "{head}");
}

#[test]
fn assets_are_cached_for_good() {
    let sb = docs_with_remote("");
    let server = sb.server(&[]);
    let page = server.get("/").body;
    let at = page.find("/assets/htmx.min.js?v=").unwrap();
    let asset = page.get(at..).unwrap().split('"').next().unwrap();
    assert!(server.get(asset).head.to_lowercase().contains("immutable"));
}

/// A page is in the language its visitor accepts, else English, whatever the server's own: it says which, and
/// that it depends on the header.
#[test]
fn pages_are_in_the_accepted_language() {
    let sb = docs_with_remote("");
    let server = sb.server(&[("LANG", "de_DE.UTF-8")]);
    for accepted in ["fr-FR,fr;q=0.9", "en-GB", "*"] {
        let page = server.get_with("/", &[&format!("Accept-Language: {accepted}")]);
        let head = page.head.to_lowercase();
        assert!(head.contains("content-language: en") && head.contains("vary: accept-language"), "{head}");
        assert!(page.body.contains(r#"<html lang="en">"#), "{accepted}");
    }
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

/// Only a browser on this computer, logged in, gets a button that opens the saved profiles' folder: for a
/// script or a reverse proxy, it would open on the server's desktop.
#[test]
fn the_profile_folder_opens_for_this_computer_only() {
    let sb = Sandbox::new();
    let server = sb.server(&[]);
    let form = "Content-Type: application/x-www-form-urlencoded";
    let login = server.request("POST", "/login", &[form], &format!("token={}", server.token));
    let session = login.head.lines().find_map(|l| l.strip_prefix("set-cookie: ")).unwrap();
    let cookie = format!("Cookie: {}", session.split(';').next().unwrap());
    let (docs, repo) = (sb.path("docs"), sb.path("repos"));
    let setup = |name: &str| {
        let repo = repo.join(name);
        format!(
            "name={name}&source.0={}&primary=local&primary.local.repository={}&primary.password=pw&target=none&every=manual&check=manual&keep=all&notify=",
            docs.display(),
            repo.display()
        )
    };
    let button = r#"hx-post="/setup/folder""#;
    let here = server.request("POST", "/setup", &[form, &cookie], &setup("Here"));
    assert!(here.body.contains(button), "{}", here.body);
    let proxied = server.request("POST", "/setup", &[form, &cookie, "X-Forwarded-For: 192.0.2.1"], &setup("Proxied"));
    assert!(proxied.code == 200 && !proxied.body.contains(button), "{}", proxied.body);
    // A Cloudflare Tunnel connects from this computer too, with the visitor's address.
    let tunneled =
        server.request("POST", "/setup", &[form, &cookie, "Cf-Connecting-Ip: 192.0.2.1"], &setup("Tunneled"));
    assert!(tunneled.code == 200 && !tunneled.body.contains(button), "{}", tunneled.body);
    let scripted = server.post_form("/setup", &setup("Scripted"));
    assert!(scripted.code == 200 && !scripted.body.contains(button), "{}", scripted.body);
    // Elsewhere, the folder's path is there to copy.
    let dir = sb.profile_path("x").parent().unwrap().display().to_string();
    assert!(scripted.body.contains(&format!(r#"data-copy="{dir}""#)), "{}", scripted.body);
    // Asked anyway, it opens nothing.
    assert_eq!(server.post_form("/setup/folder", "").code, 404);
}

#[test]
fn a_first_user_is_shown_where_to_start() {
    let sb = Sandbox::new();
    let server = sb.server(&[]);
    let page = server.get("/").body;
    let dir = sb.profile_path("x").parent().unwrap().display().to_string();
    assert!(page.contains(&dir) && page.contains("/setup"), "{page}");
    // As JSON writes it, with Windows's backslashes doubled.
    let listed = json!(dir).to_string();
    sb.run(&["backup", "--all"])
        .assert_exit(2)
        .assert_result("no-profiles", json!({}))
        .assert_output(listed.trim_matches('"'));
    // A generated password, and a field for each value of each kind of repository.
    let form = server.get("/setup").body;
    assert!(
        form.contains(r#"name="primary.password" id="setup-primary-password" value=""#)
            && form.contains(r#"name="primary.s3.bucket""#)
            && form.contains(r#"name="target.s3.bucket""#),
        "{form}"
    );
    let (docs, local, usb) = (sb.path("docs"), sb.path("repos/local"), sb.path("repos/usb"));
    let setup = format!(
        "name=Docs&source.0={}&primary=local&primary.local.repository={}&primary.password=pw&target=local&target.local.repository={}&target.password=pw&every=daily&notify=",
        docs.display(),
        local.display(),
        usb.display()
    );
    let saved = server.post_form("/setup", &setup);
    assert!(saved.body.contains("restore --all") && !saved.body.contains("password = "), "{}", saved.body);
    assert!(sb.profile_path("Docs-local").is_file() && sb.profile_path("Docs-local-copy").is_file());
    // Listed once read, even if its first backup fails.
    wait_for(15, "the listing", || {
        let page = server.get("/").body;
        page.contains("/collections/Docs-local") && sb.store().inventory("Docs-local", "Docs-local").is_some()
    });
    assert!(flash(&server.post_form("/setup", &setup).body, "failure"));
}

#[test]
fn a_file_is_found_across_snapshots_and_changes_are_shown() {
    let sb = docs_with_remote("");
    sb.write("docs/sub/report.txt", "draft");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.write("docs/sub/report.txt", "final");
    sb.write("docs/notes.txt", "new");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let server = sb.server(&[]);
    let ids = sb.snapshot_ids("Docs-remote");
    let docs = crate::common::in_tree(&sb.path("source/docs"));
    // A name, in any case, anywhere in the path: each version, with a download link.
    let found = server.get("/collections/Docs/repos/Docs-remote/find?name=REPORT").body;
    assert_eq!(found.matches(&format!("<code>{docs}/sub/report.txt</code>")).count(), 2, "{found}");
    assert!(found.contains(&format!("/snapshots/{}/download?path=", ids[1].get(..8).unwrap())), "{found}");
    assert!(!server.get("/collections/Docs/repos/Docs-remote/find?name=nowhere").body.contains("/download?"));
    // What the newer snapshot changed, from the listing the copy stored.
    let changes = server.get(&format!("/collections/Docs/repos/Docs-remote/snapshots/{}/diff", ids[1])).body;
    // Page titles name what they show, then where it is, up to the collection.
    let short = ids[1].get(..8).unwrap();
    assert!(found.contains(&format!(" | Docs-remote | Docs | {APP_NAME}</title>")), "{found}");
    assert!(changes.contains(&format!(" | {short} | Docs-remote | Docs | {APP_NAME}</title>")), "{changes}");
    let row = |path: &str| changes.split("<tr>").find(|r| r.contains(&format!("<code>{docs}{path}</code>"))).unwrap();
    // Changed: as it was, and as it is. Added: as it is.
    assert_eq!(row("/sub/report.txt").matches("/download?").count(), 2);
    assert_eq!(row("/notes.txt").matches("/download?").count(), 1);
    // The first snapshot has nothing to compare with.
    let first = server.get(&format!("/collections/Docs/repos/Docs-remote/snapshots/{}/diff", ids[0])).body;
    let meta = first.split(r#"<div class="meta">"#).nth(1).unwrap().split("</div>").next().unwrap();
    assert!(!meta.contains("<a "), "{first}");
    // A test restore of one of these snapshots.
    let form = format!("kind=test-restore&repo=Docs-remote&snapshot={}", ids[0]);
    let started = server.post_form("/collections/Docs/runs", &form).body;
    assert!(flash(&started, "success"), "{started}");
    let tested = json!({"kind": "test-restored", "repo": "Docs-remote", "snapshot": ids[0].get(..8).unwrap()});
    wait_for(60, "the test", || {
        sb.store().runs("Docs", 1)[0].all_results().any(|l| crate::common::has(&json!(l), &tested))
    });
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
    assert_eq!(sb.queued("Slow"), 2);
    drop(server);
    // Restarted: the waiting requests run.
    let _server = sb.server(&[]);
    let tested =
        || sb.store().runs("Slow", usize::MAX).iter().any(|r| json!(r.kind) == "test-restore" && r.finished.is_some());
    wait_for(60, "the test restore", tested);
    wait_for(90, "both to end", || sb.queued("Slow") == 0);
}

#[test]
fn a_fresh_state_is_filled_on_first_boot() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.remove("store");
    let server = sb.server(&[]);
    wait_for(30, "the listings", || {
        let profile = server.collection("Docs");
        profile["last_backup"].is_string() && profile["targets"][0]["lag"]["checked_at"].is_string()
    });
    let profile = server.collection("Docs");
    assert!(!json_has(&profile["issues"].to_string(), &json!({"kind": "never-backed-up"})), "{profile}");
    assert!(card(&server.get("/collections/Docs").body, "repos").contains(r#"<td class="right num">1</td>"#));
}

/// Monitors read `/metrics`: a renamed metric or label breaks their queries.
#[test]
fn metrics_keep_their_names() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let server = sb.server(&[]);
    assert_eq!(server.request("GET", "/metrics", &[], "").code, 401);
    let filters = [
        (r"(_seconds(?:\{[^}]*\})?) [\d.]+", "$1 [time]"),
        (r"(_bytes\{[^}]*\}) \d+", "$1 [bytes]"),
        (r#"version="[^"]+""#, r#"version="[version]""#),
    ];
    insta::with_settings!({filters => filters}, {
        insta::assert_snapshot!(server.get("/metrics").body);
    });
}

/// Answers every request with `204 No Content`, and counts them.
fn receiver() -> (String, std::sync::mpsc::Receiver<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("json://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        for mut socket in listener.incoming().flatten() {
            let mut request = [0; 8192];
            _ = socket.set_read_timeout(Some(Duration::from_secs(5)));
            _ = socket.read(&mut request);
            _ = write!(socket, "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            _ = tx.send(());
        }
    });
    (url, rx)
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
