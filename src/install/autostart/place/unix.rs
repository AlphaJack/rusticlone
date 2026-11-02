//! Unix's links, through which Homebrew puts its executables on `PATH`.

#![expect(clippy::disallowed_methods, reason = "lays out a Homebrew folder")]

use super::{
    kept, linked,
    tests::{Folder, path},
};

/// As Homebrew's `bin/rusticlone` leads to its `Cellar`: the link outlives the version's folder.
#[test]
fn homebrews_executable_stays_named_by_its_link_on_path() {
    let folder = Folder::new("linked");
    let exe = folder.exe("Cellar/rusticlone/2.0.1/bin/rusticlone");
    let bin = folder.0.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink(&exe, bin.join(crate::NAME)).unwrap();
    let on_path = path(&[&folder.0.join("elsewhere"), &bin]);
    assert_eq!(linked(&exe, &on_path), Some(bin.join(crate::NAME)));
    assert_eq!(kept(&exe, &on_path, None), Some(bin.join(crate::NAME)));
    // Started through the link, as macOS then names it.
    assert_eq!(kept(&bin.join(crate::NAME), &on_path, None), Some(bin.join(crate::NAME)));
    // Without a link on `PATH`, where the package manager keeps it, rather than a copy.
    assert_eq!(kept(&exe, &path(&[&folder.0.join("elsewhere")]), None), Some(exe));
}
