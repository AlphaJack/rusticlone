//! Repositories listed and browsed: snapshots, files and downloads.

use super::*;

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
    // Without a [forget] policy, neither repository has anything to prune.
    assert_eq!(card(&server.get("/collections/Docs").body, "repos").matches("keeps every snapshot").count(), 2);
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
    // A download ends with its last chunk only once rustic succeeded: else it is cut short.
    let complete = |body: &str| body.ends_with("0\r\n\r\n");
    assert!(complete(&file.body) && complete(&zip.body), "{:?}", file.body);
    let failed = server.get(&format!("{base}/download?path={docs}/nowhere.txt"));
    assert!(!complete(&failed.body), "{:?}", failed.body);
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
    #[cfg(unix)]
    let env = crate::common::wrapped_rustic(&sb, "ls", "sleep 2");
    #[cfg(windows)]
    let env = {
        let path = std::env::var_os("PATH").unwrap();
        let rustic = std::env::split_paths(&path).map(|d| d.join("rustic.exe")).find(|p| p.is_file()).unwrap();
        let dir = sb.path("slow");
        std::fs::create_dir_all(&dir).unwrap();
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
    // The older snapshot, from the same listing, among the others.
    let older = server.get(&format!("/collections/Docs/repos/Docs-remote/snapshots/{}", ids[0])).body;
    let short = ids[0].get(..8).unwrap();
    assert!(older.contains(&format!("<title>{short} | Docs-remote | Docs | {APP_NAME}</title>")), "{older}");
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
