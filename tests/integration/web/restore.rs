//! Restores, test restores and re-creating lost repositories from the web UI.

use super::*;

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
    // Without a choice, the dialog offers every folder, each backup entry, or a subfolder.
    let choose = new("kind=restore").body;
    assert!(
        choose.contains(r#"<option value="all">"#) && choose.contains(&format!(r#"<option value="0">{docs}</option>"#)),
        "{choose}"
    );
    assert!(choose.contains(r#"<option value="custom">"#) && choose.contains(r#"type="datetime-local""#), "{choose}");
    assert_eq!(new("kind=backup").code, 404);
    // In the page's dialog, only the form is returned.
    let dialog = server.get_with("/collections/Docs/runs/new?kind=restore", &["HX-Request: true"]).body;
    assert!(dialog.trim_start().starts_with("<form") && !dialog.contains("<title>"), "{dialog}");
    let form = format!("kind=restore&repo=Docs-remote&snapshot={id}&path={docs}/sub&to={}", to.display());
    assert!(flash(&server.post_form("/collections/Docs/runs", &form).body, "success"));
    wait_for(30, "the restore", || sb.store().runs("Docs", 1).first().is_some_and(|r| r.finished.is_some()));
    assert_eq!(std::fs::read_to_string(to.join("sub").join("b.txt")).unwrap(), "inside");
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
    // Nothing to restore from: no Restore button.
    assert!(!card(&page, "sources").contains(r##"<use href="#restore"/>"##), "{page}");
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
    wait_for(10, "the status", || repos().contains(r##"<use href="#recreate"/>"##));
    // Re-create is offered, and there is no Copy from a primary that is gone.
    assert!(!repos().contains(r##"<use href="#copy"/>"##));
    assert_eq!(server.post_form("/collections/Docs/repos/Docs/runs", "kind=copy").code, 422);
    // The form offers the target with the newest snapshots, or another one.
    let form = server.get("/collections/Docs/runs/new?kind=re-create&repo=Docs").body;
    assert!(form.contains(r#"<option value="">"#), "{form}");
    assert!(form.contains(r#"<option value="Docs-remote">Docs-remote</option>"#), "{form}");
    assert!(form.contains("the copy target with the newest snapshots"), "{form}");
    // A target comes back from the primary, not from itself.
    let target = server.get("/collections/Docs/runs/new?kind=re-create&repo=Docs-remote").body;
    assert!(
        target.contains("the primary: re-created empty") && !target.contains(r#"<option value="Docs">"#),
        "{target}"
    );
    server.post_form("/collections/Docs/repos/Docs/runs", "kind=re-create&from=Docs-remote");
    wait_for(30, "the primary to come back", || sb.snapshots("Docs").len() == 1);
    // Copied by the backup, or by the server first: the chunker matches either way.
    sb.write("docs/a.txt", "version 3");
    sb.run(&["backup", "--all"]).assert_exit(0);
    wait_for(30, "the copy", || sb.snapshots("Docs-remote").len() == 2);
}
