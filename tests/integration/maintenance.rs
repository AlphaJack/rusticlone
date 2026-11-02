use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
};

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

/// The files under `dir`, read as they are, even while a prune adds and removes them.
fn files(dir: &Path) -> BTreeSet<PathBuf> {
    let mut found = BTreeSet::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files(&path));
        } else {
            found.insert(path);
        }
    }
    found
}

/// A prune killed at any point leaves a consistent repository: it writes repacked data before it
/// deletes the old data, which it removes a day later. The next prune removes what remains. Rustic
/// options that are not passed (`--early-delete-index`) could change that. This test
/// repacks every pack, the most a prune can do.
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
    // Small packs, so that the prune has many to repack, and so many steps to be stopped at.
    assert!(ok(&["init"]) && ok(&["config", "--set-datapack-size", "1MiB", "--set-datapack-size-limit", "1MiB"]));
    assert!(ok(&["backup"]));
    write(12);
    assert!(ok(&["backup"]) && ok(&["forget", "--keep-last", "1"]));
    let (repo, kept) = (sb.path("local/Docs"), sb.path("kept"));
    let copy = |from: &Path, to: &Path| {
        _ = fs::remove_dir_all(to);
        assert!(std::process::Command::new("cp").arg("-a").arg(from).arg(to).status().unwrap().success());
    };
    copy(&repo, &kept);
    // Each prune is stopped by the files it added or removed, not by time, which another machine's speed and load
    // change. The files are listed, not watched: kqueue reports a new file as a change to its directory.
    let before = files(&repo);
    let changed = || before.symmetric_difference(&files(&repo)).count();
    let prune = || {
        sb.command("rustic")
            .args(["-P", "Docs", "prune", "--repack-all"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    };
    let mut whole = prune();
    assert!(whole.wait().unwrap().success());
    let total = changed();
    assert!(total > 2, "the prune changed {total} files");
    // A prune can end between two listings, slow on Windows, so it is tried until three were stopped.
    let (mut stopped, mut tried) = (0, 0);
    while stopped < 3 && tried < 15 {
        // After a sixth of its changes in the first half, so that the stops are at other steps of the prune.
        let sixth = tried % 5 + 1;
        tried += 1;
        copy(&kept, &repo);
        let mut pruning = prune();
        let changes = (total * sixth / 12).max(1);
        while pruning.try_wait().unwrap().is_none() && changed() < changes {
            std::thread::yield_now();
        }
        _ = pruning.kill();
        if pruning.wait().unwrap().success() {
            continue;
        }
        stopped += 1;
        // A killed prune could lose a pack or index. `check` verifies that every referenced one exists.
        assert!(ok(&["check"]), "killed at {sixth}/6: the repository is damaged");
        assert!(ok(&["prune"]) && ok(&["check"]), "killed at {sixth}/6: the next prune failed");
    }
    assert!(stopped >= 3, "only {stopped} of {tried} prunes were stopped before they ended");
}
