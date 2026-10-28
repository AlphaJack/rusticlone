use std::fs;

use serde_json::json;

use crate::common::{Sandbox, cmd, docs_with_remote, toml_str};

#[test]
fn lost_target_is_recreated_with_full_history() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.write("docs/a.txt", "version 2");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.remove("remote/Docs");
    // Unavailable or lost: a copy cannot tell, so re-creating it needs a request.
    sb.run(&["copy", "--all"]).assert_exit(1).assert_result("failed", json!({}));
    assert!(!sb.path("remote/Docs").exists());
    sb.run(&["recreate", "--repo", "Unknown"]).assert_exit(1).assert_result("error", json!({}));
    sb.run(&["recreate", "--repo", "Docs-remote"]).assert_exit(0).assert_result("copied", json!({"snapshots": 2}));
    assert_eq!(sb.snapshots("Docs-remote"), sb.snapshots("Docs"));
    sb.assert_stored_matches("Docs", &["Docs", "Docs-remote"]);
}

#[test]
fn failing_target_shows_as_lag_without_blocking_others() {
    let sb = docs_with_remote("");
    fs::write(sb.path("remote/unplugged"), "").unwrap();
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-usb", "Docs-remote"], "");
    sb.target("Docs-usb", "unplugged/Docs", "");
    // The copy failed, so the run did; the collection is only degraded: the backup is safe in the primary.
    sb.run(&["backup", "--all"])
        .assert_exit(1)
        .assert_result("failed", json!({}))
        .assert_result("copied", json!({"target": "Docs-remote", "snapshots": 1}));
    sb.run(&["collections"])
        .assert_exit(2)
        .assert_result("copy-failing", json!({}))
        .assert_result("copy-failed", json!({}));
}

#[test]
fn independently_initialised_target_is_reported() {
    let sb = docs_with_remote("");
    // Each `init` picks random chunker parameters, so rustic refuses to copy (and exits 0).
    assert!(sb.rustic(&["-P", "Docs", "init"]).status.success());
    assert!(sb.rustic(&["-P", "Docs-remote", "init"]).status.success());
    let run = sb.run(&["backup", "--all"]);
    run.assert_exit(1);
    run.assert_result("failed", json!({}));
    // Rustic's own message, although it exits 0.
    run.assert_output("different chunker parameter");
}

#[test]
fn remote_target_is_created_on_first_copy() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-cloud"], "");
    sb.profile(
        "Docs-cloud",
        &format!(
            "[global]\nuse-profiles = ['common']\n\n[repository]\nrepository = 'opendal:fs'\n\n[repository.options]\nroot = '{}'\n",
            sb.path("remote/cloud").display()
        ),
    );
    sb.write("docs/a.txt", "version 1");
    sb.run(&["backup", "--all"])
        .assert_exit(0)
        .assert_result("copied", json!({"target": "Docs-cloud", "snapshots": 1}));
    assert_eq!(sb.snapshots("Docs-cloud").len(), 1);
    // The primary repository can be re-created from it.
    sb.remove("local");
    sb.run(&["recreate", "--all"]).assert_exit(0).assert_result("recreated", json!({"from": "Docs-cloud"}));
}

#[test]
fn target_mounted_by_repository_hooks() {
    let sb = Sandbox::new();
    let (disk, mount) = (sb.path("disk"), sb.path("remote/usb"));
    fs::create_dir(&disk).unwrap();
    let hooks = format!(
        "[repository.hooks]\nrun-before = [{}]\nrun-finally = [{}]\n",
        toml_str(&cmd::link(&disk, &mount)),
        toml_str(&cmd::unlink(&mount))
    );
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-usb"], "");
    sb.target("Docs-usb", "usb/Docs", &hooks);
    sb.write("docs/a.txt", "version 1");
    sb.run(&["backup", "--all"]).assert_exit(0).assert_result("copied", json!({"target": "Docs-usb", "snapshots": 1}));
    assert!(!mount.exists() && disk.join("Docs/config").is_file());
    assert_eq!(sb.snapshots("Docs-usb").len(), 1);
}

#[test]
fn target_with_shorter_history_gets_no_recopies() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-remote"], "[forget]\nkeep-last = 10\n");
    sb.target("Docs-remote", "Docs", "[forget]\nkeep-last = 1\n");
    for version in 1..=3 {
        sb.write("docs/a.txt", &format!("version {version}"));
        sb.run(&["backup", "--all"]).assert_exit(0).assert_result("copied", json!({"snapshots": 1}));
    }
    sb.run(&["copy", "--all"]).assert_exit(0).assert_result("in-sync", json!({}));
    assert_eq!(sb.snapshots("Docs").len(), 3);
    assert_eq!(sb.snapshots("Docs-remote").len(), 1);
}

#[test]
fn profile_that_is_also_a_copy_target() {
    let sb = Sandbox::new();
    sb.backup_profile("A", &[&["a"]], &["B"], "");
    sb.backup_profile("B", &[&["b"]], &[], "");
    sb.write("a/file", "a");
    sb.write("b/file", "b");
    // B's repository exists first, so A's new repository silently takes its chunker.
    sb.run(&["backup", "--collection", "B"]).assert_exit(0);
    sb.run(&["backup", "--collection", "A"])
        .assert_exit(0)
        .assert_result("primary-created", json!({"from": "B"}))
        .assert_result("copied", json!({"target": "B", "snapshots": 1}));
    sb.run(&["collections"]).assert_result("backup-profile", json!({"name": "B", "copy_target_of": ["A"]}));
}

#[test]
fn unchanged_snapshots_a_target_drops_are_not_copied_again() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-remote"], "");
    sb.target("Docs-remote", "Docs", "[forget]\nkeep-last = 10\ndelete-unchanged = true\n");
    sb.write("docs/a.txt", "version 1");
    sb.run(&["backup", "--all"]).assert_exit(0).assert_result("copied", json!({"snapshots": 1}));
    // Nothing changed: the target's own forget removes the copy, which is never sent again.
    sb.run(&["backup", "--all"])
        .assert_exit(0)
        .assert_result("copied", json!({"target": "Docs-remote", "snapshots": 1}))
        .assert_result("forgot", json!({"repo": "Docs-remote", "snapshots": 1}));
    sb.run(&["copy", "--all"]).assert_exit(0).assert_result("in-sync", json!({"target": "Docs-remote"}));
    assert_eq!((sb.snapshots("Docs").len(), sb.snapshots("Docs-remote").len()), (2, 1));
    sb.assert_stored_matches("Docs", &["Docs", "Docs-remote"]);
    sb.run(&["collections", "--refresh"]).assert_exit(0).assert_result("in-sync", json!({}));
    sb.write("docs/a.txt", "version 2");
    sb.run(&["backup", "--all"]).assert_exit(0).assert_result("copied", json!({"snapshots": 1}));
}

/// A copy target without a collection: the backup is saved, and the run fails with the reason.
#[test]
fn copy_target_without_profile_fails_the_run() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-offsite"], "");
    sb.write("docs/a.txt", "a");
    sb.run(&["backup", "--all"])
        .assert_exit(1)
        .assert_result("copy-target-unusable", json!({"target": "Docs-offsite", "broken": false}));
    assert_eq!(sb.snapshots("Docs").len(), 1);
    let run = &sb.store().runs("Docs", 1)[0];
    assert_eq!(run.status, rusticlone::Status::Failed, "{:?}", run.results);
    assert!(run.steps.iter().any(|s| s.label.contains("Docs-offsite")), "{:?}", run.steps);
}

/// A primary is re-created only once lost: empty without copy targets, and the backups continue into it.
/// Locks are named after the repository's id, which a re-created repository changes.
#[test]
fn a_re_created_primary_is_locked_by_its_new_id() {
    let sb = docs_with_remote("");
    let id = || {
        let config: serde_json::Value =
            serde_json::from_slice(&sb.rustic(&["-P", "Docs", "cat", "config"]).stdout).unwrap();
        config["id"].as_str().unwrap().to_owned()
    };
    let key = sb.path("local/Docs").display().to_string();
    sb.run(&["backup", "--all"]).assert_exit(0);
    let lost = id();
    assert_eq!(sb.store().repo_id(&key), Some(lost.clone()));
    sb.remove("local/Docs");
    sb.run(&["recreate", "--all"]).assert_exit(0).assert_result("recreated", json!({"from": "Docs-remote"}));
    sb.write("docs/a.txt", "version 2");
    sb.run(&["backup", "--all"]).assert_exit(0);
    assert_ne!(id(), lost);
    assert_eq!(sb.store().repo_id(&key), Some(id()));
}

#[test]
fn a_lost_primary_without_targets_is_re_created_empty() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &[], "");
    sb.write("docs/a.txt", "a");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.run(&["recreate", "--all"]).assert_exit(0).assert_result("listable", json!({"repo": "Docs"}));
    sb.remove("local/Docs");
    sb.run(&["backup", "--all"]).assert_exit(1).assert_result("failed", json!({}));
    sb.run(&["recreate", "--all"]).assert_exit(0).assert_result("primary-recreated-empty", json!({}));
    sb.run(&["backup", "--all"]).assert_exit(0);
    assert_eq!(sb.snapshots("Docs").len(), 1);
}

/// Re-created from the chosen repository: the primary from a target that is not the newest, a
/// target from another target, but never from itself.
#[test]
fn a_repository_is_re_created_from_the_one_chosen() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-usb", "Docs-remote"], "");
    sb.target("Docs-usb", "usb/Docs", "");
    sb.target("Docs-remote", "Docs", "");
    sb.write("docs/a.txt", "version 1");
    sb.run(&["backup", "--all"]).assert_exit(0);
    // The USB disk is unplugged for the second backup: it has one snapshot, the remote two.
    fs::rename(sb.path("remote/usb"), sb.path("usb-in-a-drawer")).unwrap();
    sb.write("docs/a.txt", "version 2");
    sb.run(&["backup", "--all"]).assert_exit(1);
    fs::rename(sb.path("usb-in-a-drawer"), sb.path("remote/usb")).unwrap();

    sb.remove("local/Docs");
    sb.run(&["recreate", "--all", "--from", "Docs-usb"])
        .assert_exit(0)
        .assert_result("recreated", json!({"from": "Docs-usb", "snapshots": 1}));
    assert_eq!(sb.snapshots("Docs").len(), 1);

    sb.remove("remote/Docs");
    sb.run(&["recreate", "--repo", "Docs-remote", "--from", "Docs-usb"])
        .assert_exit(0)
        .assert_result("recreated", json!({"repo": "Docs-remote", "from": "Docs-usb", "snapshots": 1}));
    assert_eq!(sb.snapshots("Docs-remote").len(), 1);
    sb.assert_stored_matches("Docs", &["Docs", "Docs-remote"]);
    sb.run(&["recreate", "--repo", "Docs-usb", "--from", "Docs-usb"]).assert_exit(1).assert_result("error", json!({}));
}
