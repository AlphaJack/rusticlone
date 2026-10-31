//! The emergency kit on a new computer: no profiles, no state and no sources. The repositories are where they
//! were. `restore --all --kit` brings the files back from what is left of them.

use std::{fs, path::PathBuf};

use serde_json::json;

use crate::common::{Sandbox, as_root, docs_with_remote};

/// The kit that the web UI downloads, saved in the sandbox.
fn download(sb: &Sandbox) -> PathBuf {
    let server = sb.server(&[]);
    let kit = server.get("/kit");
    assert!(kit.head.contains("restore-kit.zip"), "{}", kit.head);
    let path = sb.path("restore-kit.zip");
    fs::write(&path, kit.bytes).unwrap();
    path
}

/// A computer that has only the repositories: its profiles, state, cache and sources are gone.
fn new_computer(sb: &Sandbox) {
    for entry in fs::read_dir(sb.profile_dir()).unwrap().flatten() {
        fs::remove_file(entry.path()).unwrap();
    }
    for dir in ["store", "cache"] {
        _ = fs::remove_dir_all(sb.path(dir));
    }
    sb.remove("source");
}

/// A backup of `docs` copied to `Docs-remote`, its kit, and a new computer.
fn backed_up() -> (Sandbox, PathBuf, std::collections::BTreeMap<PathBuf, Vec<u8>>) {
    let sb = docs_with_remote("");
    sb.write("docs/sub/b.txt", "b");
    sb.run(&["backup", "--all"]).assert_exit(0);
    let original = sb.tree("source");
    let kit = download(&sb);
    new_computer(&sb);
    (sb, kit, original)
}

fn restore(sb: &Sandbox, kit: &std::path::Path) -> crate::common::Run {
    sb.run(&["restore", "--all", "--kit", &kit.display().to_string()])
}

#[test]
fn a_kit_restores_everything_on_a_new_computer() {
    // Root saves the kit into the system folder, `/etc/rustic`, which no sandbox redirects.
    if as_root() {
        return;
    }
    let (sb, kit, original) = backed_up();
    // Every folder is back where it was: the profiles fit this computer, so no warning.
    restore(&sb, &kit)
        .assert_exit(0)
        .assert_result("restored", json!({"repo": "Docs"}))
        .assert_no_result("profiles-elsewhere", json!({}));
    assert_eq!(sb.tree("source"), original);
    // The profiles are where rustic finds them: backups go on.
    sb.write("docs/c.txt", "c");
    sb.run(&["backup", "--all"]).assert_exit(0);
}

#[test]
fn a_kit_restores_from_the_copy_when_the_primary_is_gone() {
    // Root saves the kit into the system folder, `/etc/rustic`, which no sandbox redirects.
    if as_root() {
        return;
    }
    let (sb, kit, original) = backed_up();
    sb.remove("local/Docs");
    restore(&sb, &kit).assert_exit(0).assert_result("recreated", json!({"from": "Docs-remote"}));
    assert_eq!(sb.tree("source"), original);
}

#[test]
fn a_kit_restores_from_the_primary_when_the_copy_is_gone() {
    // Root saves the kit into the system folder, `/etc/rustic`, which no sandbox redirects.
    if as_root() {
        return;
    }
    let (sb, kit, original) = backed_up();
    sb.remove("remote/Docs");
    let run = restore(&sb, &kit);
    run.assert_result("restored", json!({"repo": "Docs"}));
    assert_eq!(sb.tree("source"), original, "{}", run.stdout);
}

/// The primary's data is damaged, its listing is not: the copy still holds the files.
#[test]
fn a_kit_restores_from_the_copy_when_the_primary_is_corrupted() {
    // Root saves the kit into the system folder, `/etc/rustic`, which no sandbox redirects.
    if as_root() {
        return;
    }
    let (sb, kit, original) = backed_up();
    for pack in walk(&sb.path("local/Docs/data")) {
        fs::write(pack, b"damaged").unwrap();
    }
    let run = restore(&sb, &kit);
    assert_eq!(sb.tree("source"), original, "{}", run.stdout);
    run.assert_result("falling-back", json!({"repo": "Docs", "to": "Docs-remote"}))
        .assert_result("restored", json!({"repo": "Docs-remote"}));
}

/// The disk is mounted elsewhere on the new computer: the restore says which profile names the old place.
#[test]
fn a_kit_says_where_it_looked_when_no_repository_is_there() {
    if as_root() {
        return;
    }
    let (sb, kit, _) = backed_up();
    fs::rename(sb.path("local"), sb.path("elsewhere")).unwrap();
    sb.remove("remote/Docs");
    let run = restore(&sb, &kit);
    run.assert_exit(1);
    // The kit unpacks into the working folder, as a folder of profiles in a disaster recovery.
    let (docs, file) = (sb.path("local/Docs").display().to_string(), sb.path("cwd/Docs.toml").display().to_string());
    let said = run.stdout.replace("\\\\", "\\");
    assert!(said.contains("could be read") && said.contains(&docs) && said.contains(&file), "{}", run.stdout);
    // And that these profiles do not fit this computer yet.
    run.assert_result("profiles-elsewhere", json!({}));
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
