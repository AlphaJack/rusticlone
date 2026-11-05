//! Open pages kept current: what changed is pushed, and listings follow the runs.

use super::*;

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
    // The notice names the collection whose run started.
    let started = server.post_form("/collections/Docs/runs", "kind=backup").body;
    assert!(started.contains("Docs: "), "{started}");
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
