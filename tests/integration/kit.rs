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

/// A kit whose repositories are all gone: the upload saves its profiles, the restore fails, and the collection
/// says that its backups wait and which profiles to adapt.
#[test]
fn a_failed_restore_of_an_uploaded_kit_is_shown() {
    if as_root() {
        return;
    }
    let (sb, kit, _) = backed_up();
    sb.remove("local/Docs");
    sb.remove("remote/Docs");
    let server = sb.server(&[]);
    let auth = format!("Authorization: Bearer {}", server.token);
    let saved =
        server.request_bytes("POST", "/kit", &[&auth, "Content-Type: application/zip"], &fs::read(&kit).unwrap());
    assert_eq!(saved.code, 201, "{}", saved.head);
    let mut status = String::new();
    crate::common::wait_for(60, "the failed restore", || {
        status = server.get("/status").body;
        status.contains(r#""kind":"restore-failed""#)
    });
    assert!(status.contains("Docs.toml") && !status.contains("never-backed-up"), "{status}");
}

/// As the web UI restores a kit: only where nothing is. The damaged primary writes some folders before it
/// fails, and the copy's files still go there.
#[test]
fn a_kit_restores_missing_folders_from_the_copy_when_the_primary_is_corrupted() {
    if as_root() {
        return;
    }
    let (sb, kit, original) = backed_up();
    for pack in walk(&sb.path("local/Docs/data")) {
        fs::write(pack, b"damaged").unwrap();
    }
    // A new Mac's folder: empty but for the system's file.
    fs::create_dir_all(sb.path("source/docs")).unwrap();
    fs::write(sb.path("source/docs/.DS_Store"), "").unwrap();
    let run = sb.run(&["restore", "--all", "--missing", "--kit", &kit.display().to_string()]);
    let restored: std::collections::BTreeMap<_, _> =
        sb.tree("source").into_iter().filter(|(p, _)| !p.ends_with(".DS_Store")).collect();
    assert_eq!(restored, original, "{}", run.stdout);
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
    // The new computer has the folder, so only the failed restore tells that the profile is another computer's.
    fs::create_dir_all(sb.path("source/docs")).unwrap();
    fs::write(sb.path("source/docs/.DS_Store"), "").unwrap();
    let run = restore(&sb, &kit);
    run.assert_exit(1);
    // The kit unpacks into the working folder, as a folder of profiles in a disaster recovery.
    let (docs, file) = (sb.path("local/Docs").display().to_string(), sb.path("cwd/Docs.toml").display().to_string());
    let said = run.stdout.replace("\\\\", "\\");
    assert!(said.contains("could be read") && said.contains(&docs) && said.contains(&file), "{}", run.stdout);
    // And that these profiles do not fit this computer yet.
    let elsewhere = run.stdout.lines().find(|l| l.contains("profiles-elsewhere")).unwrap_or_default();
    assert!(elsewhere.contains("Docs.toml"), "{}", run.stdout);
}

/// A fresh install's home page takes the kit: its profiles go where the setup saves them, and its collections
/// show up. They are restored first, never backed up while their folders are empty. The setup stays the main
/// way to start.
#[test]
fn a_fresh_install_takes_a_kit_upload() {
    if as_root() {
        return;
    }
    let (sb, kit, original) = backed_up();
    // Read from the repository: the profiles are gone.
    let snapshots = || walk(&sb.path("local/Docs/snapshots")).len();
    let before = snapshots();
    assert!(before > 0);
    // A new computer has the folder, empty: a backup now would save nothing over what the kit restores.
    fs::create_dir_all(sb.path("source/docs")).unwrap();
    let server = sb.server(&[]);
    let page = server.get("/").body;
    let setup = page.find(r#"class="btn primary" href="/setup""#);
    let upload = page.find(r#"data-upload="/kit""#);
    assert!(setup.is_some() && upload.is_some(), "{page}");
    let dir = sb.profile_dir().display().to_string();
    assert!(page.contains(&format!(r#"data-copy="{dir}""#)), "{page}");
    let auth = format!("Authorization: Bearer {}", server.token);
    let post = |body: &[u8]| server.request_bytes("POST", "/kit", &[&auth, "Content-Type: application/zip"], body);
    // The page opens the restore of its one collection.
    let saved = post(&fs::read(&kit).unwrap());
    assert!(saved.code == 201 && saved.head.contains("hx-location: /collections/Docs/runs/"), "{}", saved.head);
    assert!(sb.profile_path("Docs").is_file() && sb.profile_path("Docs-remote").is_file());
    crate::common::wait_for(15, "the kit's collection", || server.get("/").body.contains("/collections/Docs"));
    crate::common::wait_for(60, "the restore", || sb.tree("source") == original);
    assert_eq!(snapshots(), before, "backed up before the restore");
    // The same kit again saves nothing; anything else is refused.
    assert!(post(&fs::read(&kit).unwrap()).body.contains("already"));
    let refused = post(b"not a zip");
    assert!(refused.code == 422 && refused.body.contains("not an emergency kit"), "{}", refused.body);
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
