//! Windows's keepers: `WinGet`, Chocolatey and Scoop.

#![expect(clippy::disallowed_methods, reason = "lays out package managers' folders")]

use std::path::Path;

use super::{Keeper, keeper, managed};

#[test]
fn wingets_packages_are_managed() {
    let user = r"C:\Users\me\AppData\Local\Microsoft\WinGet\Packages\AlphaJack.Rusticlone_Source\rusticlone.exe";
    let machine = r"C:\Program Files\WinGet\Packages\AlphaJack.Rusticlone_Source\rusticlone.exe";
    assert_eq!((keeper(Path::new(user)), keeper(Path::new(machine))), (Some(Keeper::WinGet), Some(Keeper::WinGet)));
}

/// Found by its layout, as `ChocolateyInstall` may move its root anywhere.
#[test]
fn chocolateys_lib_is_managed_wherever_its_root_is() {
    let root = std::env::temp_dir().join(format!("{}-chocolatey-{}", crate::NAME, std::process::id()));
    let exe = root.join(r"lib\rusticlone\tools\rusticlone.exe");
    std::fs::create_dir_all(&root).unwrap();
    assert!(!managed(&exe));
    std::fs::write(root.join("choco.exe"), b"").unwrap();
    let found = keeper(&exe);
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(found, Some(Keeper::Chocolatey));
}

/// Found by its layout, as `SCOOP` may move its root anywhere.
#[test]
fn scoops_apps_are_managed_wherever_its_root_is() {
    let root = std::env::temp_dir().join(format!("{}-scoop-{}", crate::NAME, std::process::id()));
    let exe = root.join(r"apps\rusticlone\current\rusticlone.exe");
    assert!(!managed(&exe));
    std::fs::create_dir_all(root.join("shims")).unwrap();
    let found = keeper(&exe);
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(found, Some(Keeper::Scoop));
}
