use std::{fs, path::PathBuf};

use serde_json::json;

use crate::common::{Sandbox, below_root, docs_with_remote};

#[test]
fn restores_lost_sources_and_local_repository_from_target() {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"], &["keys.kdbx", "secrets"]], &["Docs-remote"], "");
    sb.target("Docs-remote", "Docs", "");
    sb.write("docs/nested/deep/a.txt", "a");
    sb.write("keys.kdbx", "single file source");
    sb.write("secrets/env", "b");
    let original = sb.tree("source");
    sb.run(&["backup", "--all"]).assert_exit(0);

    sb.remove("source");
    // Its disk is here, without the repository.
    sb.remove("local/Docs");
    sb.remove("cache");
    // The primary is re-created first, and the sources come from it: the target is downloaded once.
    sb.run(&["restore", "--all"])
        .assert_exit(0)
        .assert_result("recreated", json!({"from": "Docs-remote"}))
        .assert_result("restored", json!({"repo": "Docs"}))
        .assert_no_result("restored", json!({"repo": "Docs-remote"}));
    assert_eq!(sb.tree("source"), original);
    sb.assert_stored_matches("Docs", &["Docs", "Docs-remote"]);
    // Backups continue into the re-created primary repository, one snapshot per source group.
    sb.write("docs/b.txt", "b");
    sb.run(&["backup", "--all"]).assert_exit(0).assert_result("copied", json!({"snapshots": 2}));
}

/// The primary's disk is not here: the sources come from the target, and the primary is not re-created on
/// whatever disk now holds its path, which rustic would create.
#[test]
fn restores_from_the_target_while_the_primary_disk_is_away() {
    let sb = docs_with_remote("");
    sb.write("docs/a.txt", "a");
    let original = sb.tree("source");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.remove("source");
    sb.remove("local");
    sb.run(&["restore", "--all"])
        .assert_exit(2)
        .assert_result("restored", json!({"repo": "Docs-remote"}))
        .assert_result("not-recreated", json!({"repo": "Docs", "from": "Docs-remote"}))
        .assert_output("recreate --repo Docs");
    assert_eq!(sb.tree("source"), original);
    assert!(!sb.path("local").exists());
}

/// Its disks are elsewhere, as on another computer: the error names each place tried and the profile to edit.
#[test]
fn a_restore_says_where_it_looked_when_no_repository_is_there() {
    let sb = docs_with_remote("");
    sb.write("docs/a.txt", "a");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.remove("local");
    sb.remove("remote");
    sb.run(&["restore", "--all"])
        .assert_exit(1)
        .assert_output("no repository of Docs could be read")
        .assert_output("Docs-remote at ")
        .assert_output("Docs.toml");
}

/// The primary's data is damaged, its listing is not: the restore goes on from the copy, which holds the files.
#[test]
fn a_restore_falls_back_to_the_copy_when_the_primary_is_damaged() {
    let sb = docs_with_remote("");
    sb.write("docs/a.txt", "a");
    let original = sb.tree("source");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb.remove("source");
    for pack in walk(&sb.path("local/Docs/data")) {
        fs::write(pack, b"damaged").unwrap();
    }
    sb.run(&["restore", "--all"])
        .assert_result("falling-back", json!({"repo": "Docs", "to": "Docs-remote"}))
        .assert_result("restored", json!({"repo": "Docs-remote"}));
    assert_eq!(sb.tree("source"), original);
}

/// Every file below `dir`.
fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() { files.extend(walk(&path)) } else { files.push(path) }
    }
    files
}

#[test]
fn restores_into_another_directory_after_a_hostname_change() {
    let sb = Sandbox::new();
    sb.target("Docs-remote", "Docs", "");
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-remote"], "[backup]\nhost = 'old-host'\n");
    sb.write("docs/a.txt", "a");
    let original = sb.tree("source/docs");
    sb.run(&["backup", "--all"]).assert_exit(0);

    // A new computer: another hostname, nothing local.
    sb.backup_profile("Docs", &[&["docs"]], &["Docs-remote"], "[backup]\nhost = 'new-host'\n");
    sb.remove("source");
    sb.remove("local");
    let to = sb.path("restored");
    sb.run(&["restore", "--all", "--to", to.to_str().unwrap()]).assert_exit(0);
    let docs = sb.path("source/docs");
    let below = below_root(&docs);
    assert_eq!(sb.tree(&format!("restored/{}", below.display())), original);
}

#[test]
fn restores_a_snapshot_path_by_its_name() {
    let sb = docs_with_remote("");
    sb.write("docs/taxes/2026.txt", "paid");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let groups: serde_json::Value =
        serde_json::from_slice(&sb.rustic(&["-P", "Docs", "snapshots", "--json"]).stdout).unwrap();
    let id = groups[0]["snapshots"][0]["id"].as_str().unwrap();
    let (taxes, to) = (sb.path("source/docs/taxes"), sb.path("restored"));
    let (taxes, to) = (taxes.to_str().unwrap(), to.to_str().unwrap());
    sb.run(&["restore", "-c", "Docs", "--snapshot", id, "--snapshot-path", taxes, "--to", to]).assert_exit(0);
    assert_eq!(fs::read_to_string(sb.path("restored/taxes/2026.txt")).unwrap(), "paid");
}

#[test]
fn restores_a_point_in_time() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let first = sb.snapshots("Docs").remove(0);
    sb.write("docs/a.txt", "version 2 (encrypted by ransomware)");
    sb.run(&["backup", "--all"]).assert_exit(0);

    sb.run(&["restore", "--all", "--at-time", &first]).assert_exit(0);
    assert_eq!(fs::read_to_string(sb.path("source/docs/a.txt")).unwrap(), "version 1");
    sb.run(&["restore", "--all", "--at-time", "2000-01-01"]).assert_exit(1).assert_result("failed", json!({}));
}

#[test]
fn restores_from_a_chosen_repository() {
    let sb = docs_with_remote("");
    sb.run(&["backup", "--all"]).assert_exit(0);
    // The primary repository has a newer snapshot that the target lacks.
    sb.write("docs/a.txt", "version 2");
    assert!(sb.rustic(&["-P", "Docs", "backup"]).status.success());

    sb.run(&["restore", "--repo", "Docs-remote"]).assert_exit(0);
    assert_eq!(fs::read_to_string(sb.path("source/docs/a.txt")).unwrap(), "version 1");
}

/// Collection Docs with two sources, `docs` and `mail`, backed up once.
fn docs_and_mail_backed_up() -> Sandbox {
    let sb = Sandbox::new();
    sb.backup_profile("Docs", &[&["docs"], &["mail"]], &[], "");
    sb.write("docs/a.txt", "a");
    sb.write("mail/m.txt", "m");
    sb.run(&["backup", "--all"]).assert_exit(0);
    sb
}

/// Safe at every boot: existing paths stay as they are, even if changed since; missing ones come back.
#[test]
fn restoring_what_is_missing_leaves_the_rest() {
    let sb = docs_and_mail_backed_up();
    sb.write("docs/a.txt", "changed since");
    sb.remove("source/mail");
    sb.run(&["restore", "--all", "--missing"]).assert_exit(0).assert_result("left-as-it-is", json!({}));
    assert_eq!(fs::read_to_string(sb.path("source/docs/a.txt")).unwrap(), "changed since");
    assert_eq!(fs::read_to_string(sb.path("source/mail/m.txt")).unwrap(), "m");
}

/// One source of several is restored, by one of its paths; a path that no source has is refused.
#[test]
fn one_source_is_restored() {
    let sb = docs_and_mail_backed_up();
    sb.remove("source/docs");
    sb.remove("source/mail");
    let docs = sb.path("source/docs").display().to_string();
    sb.run(&["restore", "--all", "--source", &docs]).assert_exit(0);
    assert!(sb.path("source/docs/a.txt").exists() && !sb.path("source/mail").exists());
    sb.run(&["restore", "--all", "--source", "/nowhere"]).assert_exit(1).assert_result("error", json!({}));
}
