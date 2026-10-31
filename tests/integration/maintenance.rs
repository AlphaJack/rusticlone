use std::{fs, process::Stdio, sync::mpsc};

use serde_json::json;

use crate::common::{KEEP, Sandbox, docs_with_remote, noise};

#[test]
fn prune_runs_on_local_and_target_repositories() {
    let sb = docs_with_remote("[forget]\nkeep-last = 1\n");
    sb.target("Docs-remote", "Docs", KEEP);
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.write("docs/a.txt", "version 2");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.run(&["prune", "--all"])
        .assert_exit(0)
        .assert_result("pruned", json!({"repo": "Docs", "freed": {}}))
        .assert_result("pruned", json!({"repo": "Docs-remote"}));
    sb.assert_stored_matches("Docs", &["Docs", "Docs-remote"]);
}

/// An append-only target is never pruned: without a `[forget]` policy, it keeps every snapshot, so its prune
/// step is skipped and says why.
#[test]
fn a_target_without_a_forget_policy_is_never_pruned() {
    let sb = docs_with_remote(KEEP);
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.run(&["prune", "--collection", "Docs"])
        .assert_exit(0)
        .assert_result("pruned", json!({"repo": "Docs"}))
        .assert_no_result("pruned", json!({"repo": "Docs-remote"}))
        .assert_result("keeps-everything", json!({"repo": "Docs-remote"}));
    sb.run(&["prune", "--repo", "Docs-remote"]).assert_exit(0).assert_result("keeps-everything", json!({}));
}

#[test]
fn prune_waits_for_a_target_that_is_behind() {
    let sb = docs_with_remote(KEEP);
    sb.target("Docs-remote", "Docs", KEEP);
    sb.run(&["backup", "--all"]).assert_exit(0);
    // A snapshot not copied yet.
    sb.write("docs/a.txt", "version 2");
    assert!(sb.rustic(&["-P", "Docs", "backup"]).status.success());
    sb.run(&["collections", "--refresh"]).assert_exit(0).assert_result("behind", json!({}));
    sb.run(&["prune", "--all"])
        .assert_exit(2)
        .assert_result("pruned", json!({"repo": "Docs"}))
        .assert_result("prune-postponed", json!({"repo": "Docs-remote"}));
}

#[test]
fn check_walks_through_all_data_subsets() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let two = &[("CHECK_SUBSETS", "2")];
    sb.common_env(two);
    sb.run(&["check", "--all"])
        .assert_exit(0)
        .assert_result("verified", json!({"repo": "Docs", "reading": {"subset": {"n": 1, "of": 2}}}))
        .assert_result("verified", json!({"repo": "Docs-remote", "reading": {"subset": {"n": 1, "of": 2}}}));
    sb.run(&["check", "--all"])
        .assert_exit(0)
        .assert_result("verified", json!({"repo": "Docs", "reading": {"subset": {"n": 2, "of": 2}}}));
    sb.run(&["collections"])
        .assert_exit(0)
        .assert_json(&json!({"name": "Docs", "next_subset": 1, "check_subsets": "2"}));
    // No subsets: only the structure is read, and the cursor stays.
    sb.common_env(&[("CHECK_SUBSETS", "0")]);
    let check = sb.run(&["check", "--all"]);
    check.assert_exit(0).assert_result("verified", json!({"reading": "structure"})).assert_no_output("--read-data");
    check.assert_output("rustic check");
    sb.common_env(two);
    sb.run(&["collections"])
        .assert_exit(0)
        .assert_json(&json!({"name": "Docs", "next_subset": 1, "check_subsets": "2"}));
    // A random sample, as rustic selects it: the cursor stays too.
    sb.common_env(&[("CHECK_SUBSETS", "1%")]);
    let check = sb.run(&["check", "--all"]);
    check
        .assert_exit(0)
        .assert_result("verified", json!({"reading": {"sample": {"percent": 100}}}))
        .assert_output("--read-data-subset 1%");
    sb.common_env(&[("CHECK_SUBSETS", "1MiB")]);
    let check = sb.run(&["check", "--all"]);
    check
        .assert_exit(0)
        .assert_result("verified", json!({"reading": {"sample": {"bytes": 1_048_576}}}))
        .assert_output("--read-data-subset 1048576");
    sb.common_env(two);
    sb.run(&["collections"])
        .assert_exit(0)
        .assert_json(&json!({"name": "Docs", "next_subset": 1, "check_subsets": "2"}));
    sb.common_env(&[("CHECK_SUBSETS", "lots")]);
    sb.run(&["check", "--all"]).assert_exit(1).assert_result("error", json!({}));
}

#[test]
fn a_target_of_several_jobs_is_checked_once() {
    let sb = Sandbox::new();
    for (profile, dir) in [("Docs", "docs"), ("Mail", "mail")] {
        sb.backup_profile(profile, &[&[dir]], &["Shared"], "");
        sb.write(&format!("{dir}/a.txt"), "a");
    }
    sb.target("Shared", "shared", "");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let out = sb.run(&["check", "--all"]);
    out.assert_exit(0);
    assert_eq!(out.stdout.matches("Shared: checked data subset").count(), 1, "{}", out.stdout);
}

/// What a watched prune does: a file it adds to or removes from the repository, or its end.
enum Did {
    Changed,
    Ended,
}

/// A prune killed at any point leaves a consistent repository: it writes repacked data before it
/// deletes the old data, which it removes a day later. The next prune removes what remains. Rustic
/// options that are not passed (`--early-delete-index`) could change that. This test
/// repacks everything unused, the most a prune can do.
#[test]
fn a_stopped_prune_leaves_the_repository_whole() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"]], &[], "");
    fs::create_dir_all(sb.path("source/docs")).unwrap();
    // Make a third of the packs unused: rewrite files, and forget their old snapshot.
    let mut written = 0;
    let mut write = |files: usize| {
        for i in 0..files {
            written += 1;
            fs::write(sb.path(&format!("source/docs/f{i}")), noise(written, 1_500_000)).unwrap();
        }
    };
    let ok = |args: &[&str]| sb.rustic(&[&["-P", "Docs"], args].concat()).status.success();
    write(24);
    assert!(ok(&["backup", "--init"]));
    write(12);
    assert!(ok(&["backup"]) && ok(&["forget", "--keep-last", "1"]));
    let (repo, kept) = (sb.path("local/Docs"), sb.path("kept"));
    let copy = |from: &std::path::Path, to: &std::path::Path| {
        _ = fs::remove_dir_all(to);
        assert!(std::process::Command::new("cp").arg("-a").arg(from).arg(to).status().unwrap().success());
    };
    copy(&repo, &kept);
    // Each prune tells what it did as it does it, so that it is stopped by what it did, not by time, which another
    // machine's speed and load change.
    let prune = || {
        let (told, did) = mpsc::channel();
        let changed = told.clone();
        let mut watcher = notify::recommended_watcher(move |e: notify::Result<notify::Event>| {
            if e.is_ok_and(|e| e.kind.is_create() || e.kind.is_remove()) {
                _ = changed.send(Did::Changed);
            }
        })
        .unwrap();
        notify::Watcher::watch(&mut watcher, &repo, notify::RecursiveMode::Recursive).unwrap();
        let mut pruning = sb
            .command("rustic")
            .args(["-P", "Docs", "prune"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        // Its output ends when it does, however it ends.
        let mut output = pruning.stdout.take().unwrap();
        std::thread::spawn(move || {
            _ = std::io::copy(&mut output, &mut std::io::sink());
            _ = told.send(Did::Ended);
        });
        (watcher, pruning, did)
    };
    copy(&kept, &repo);
    let (watcher, mut whole, did) = prune();
    assert!(whole.wait().unwrap().success());
    let total = did.iter().take_while(|d| matches!(d, Did::Changed)).count();
    drop(watcher);
    assert!(total > 2, "the prune changed {total} files");
    let mut stopped = 0;
    for sixth in 1..6 {
        // After a sixth of its changes in the first half, so that each stop is at another step of the prune.
        copy(&kept, &repo);
        let (_watcher, mut pruning, did) = prune();
        let changes = (total * sixth / 12).max(1);
        if did.iter().take(changes).any(|d| matches!(d, Did::Ended)) {
            _ = pruning.wait();
            continue;
        }
        stopped += 1;
        _ = pruning.kill();
        _ = pruning.wait();
        // A killed prune could lose a pack or index. `check` verifies that every referenced one exists.
        assert!(ok(&["check"]), "killed at {sixth}/6: the repository is damaged");
        assert!(ok(&["prune"]) && ok(&["check"]), "killed at {sixth}/6: the next prune failed");
    }
    assert!(stopped >= 3, "only {stopped} of 5 prunes were stopped before they ended");
}
