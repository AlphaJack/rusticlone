use std::fs;

use serde_json::json;

use crate::common::{Sandbox, docs_with_remote, set_readable};

#[test]
fn creates_snapshot_and_replicates_it() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"])
        .assert_exit(0)
        .assert_result("copied", json!({"target": "Docs-remote", "snapshots": 1}));
    assert_eq!(sb.snapshots("Docs").len(), 1);
    assert_eq!(sb.snapshots("Docs-remote"), sb.snapshots("Docs"));
    sb.assert_stored_matches("Docs", &["Docs", "Docs-remote"]);
}

/// The log is shown on the web page, where escape codes are noise. Rustic colors by `TERM` even into a
/// pipe, and the first backup lists repositories that do not exist yet, which it reports as `error:`.
#[test]
fn rustic_errors_are_logged_without_colors() {
    let sb = docs_with_remote("");
    sb.run_with(&["backup", "--all"], &[("TERM", "xterm")]).assert_exit(0);
    let store = sb.store();
    let (lines, _) = store.run_log_tail("Docs", store.runs("Docs", 1)[0].id, None);
    assert!(lines.iter().any(|l| l.text.contains("error:")));
    assert!(lines.iter().all(|l| !l.text.contains('\x1b')));
}

#[test]
fn creates_one_snapshot_per_source_group() {
    let sb = Sandbox::new();
    sb.backup_profile("Media", &[&["pics"], &["photos", "videos"]], &[], "");
    for file in ["pics/a", "photos/b", "videos/c"] {
        sb.write(file, file);
    }
    sb.run(&["backup", "--all"]).assert_exit(0);
    assert_eq!(sb.snapshots("Media").len(), 2);
}

/// A primary that cannot be listed may be on a disk that is not mounted yet, and a backup would fill
/// the empty mount point. Only `recreate` brings back a lost one.
#[test]
fn a_backup_never_replaces_a_primary_it_cannot_list() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.remove("local");
    fs::create_dir_all(sb.path("local/Docs")).unwrap(); // the mount point, its disk away
    sb.run(&["backup", "--all"]).assert_exit(1).assert_result("failed", json!({}));
    assert!(fs::read_dir(sb.path("local/Docs")).unwrap().next().is_none(), "nothing written into the mount point");
    sb.run(&["recreate", "--all"]).assert_exit(0).assert_result("recreated", json!({"from": "Docs-remote"}));
    // It has the target's chunker, so copies keep working.
    sb.write("docs/a.txt", "version 2");
    sb.run(&["backup", "--all"]).assert_exit(0).assert_result("copied", json!({"snapshots": 1}));
}

#[test]
fn first_backup_initialises_even_with_targets_unreachable() {
    let sb = Sandbox::new();
    fs::write(sb.path("remote/unplugged"), "").unwrap(); // the target's parent is not a directory
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-usb"], "");
    sb.target("Docs-usb", "unplugged/Docs", "");
    sb.write("docs/a.txt", "version 1");
    sb.run(&["backup", "--all"])
        .assert_exit(1)
        .assert_result("backed-up", json!({"new_repository": true}))
        .assert_result("failed", json!({}));
    assert_eq!(sb.snapshots("Docs").len(), 1);
}

#[test]
fn lost_local_repository_is_not_reinitialised_while_targets_are_unreachable() {
    let sb = Sandbox::new();
    fs::create_dir(sb.path("remote/usb")).unwrap();
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-usb"], "");
    sb.target("Docs-usb", "usb/Docs", "");
    sb.write("docs/a.txt", "version 1");
    sb.run(&["backup", "--all"]).assert_exit(0);

    // Disaster while the disk is unplugged, and the state is lost too.
    fs::rename(sb.path("remote/usb"), sb.path("usb-in-a-drawer")).unwrap();
    sb.remove("local");
    sb.run(&["backup", "--all"]).assert_exit(1).assert_result("failed", json!({}));
    assert!(!sb.path("local/Docs").exists());

    // Plugged in again: the primary is re-created from it on request.
    fs::rename(sb.path("usb-in-a-drawer"), sb.path("remote/usb")).unwrap();
    sb.run(&["recreate", "--all"]).assert_exit(0).assert_result("recreated", json!({"from": "Docs-usb"}));
}

#[test]
fn what_a_backup_could_not_read_is_reported_until_it_can() {
    let sb = docs_with_remote("");
    sb.write("docs/locked.txt", "secret");
    let locked = sb.path("source/docs/locked.txt");
    set_readable(&locked, false);
    if fs::read(&locked).is_ok() {
        // Root can read any file.
        set_readable(&locked, true);
        return;
    }
    // In the system's language on Windows.
    let denied = if cfg!(windows) { "(os error 5)" } else { "Permission denied" };
    sb.run(&["backup", "--all"]).assert_exit(2).assert_result("skipped", json!({"count": 1})).assert_output(denied);
    sb.run(&["collections"]).assert_exit(2).assert_result("skipped", json!({"count": 1}));
    set_readable(&locked, true);
    sb.write("docs/a.txt", "version 2");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.run(&["collections"]).assert_exit(0).assert_no_result("skipped", json!({}));
}

/// Rustic records nested sources as the outer one only. Its snapshot matches none of the collection's
/// sources, which no copy or restore would find: the backup fails and says how to fix the profile.
#[test]
fn a_snapshot_that_matches_no_source_fails_the_backup() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs", "docs/sub"]], &[], "");
    sb.write("docs/sub/a.txt", "a");
    sb.run(&["backup", "--all"]).assert_exit(1).assert_result("failed", json!({})).assert_output("outside the others");
}

/// A `[snapshot-filter]` that hides the new snapshot from listings would hide it from copies and restores.
#[test]
fn a_snapshot_filter_that_hides_the_backup_fails_it() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &[], "[snapshot-filter]\nfilter-hosts = ['elsewhere']\n");
    sb.write("docs/a.txt", "a");
    sb.run(&["backup", "--all"]).assert_exit(1).assert_result("failed", json!({})).assert_output("[snapshot-filter]");
}
