//! Executables put on this computer: rustic's release when it is missing, rusticlone's own on update,
//! the folder they are in on `PATH`, and the server started with the computer.

#[cfg(feature = "self-update")]
mod attest;
#[cfg(feature = "desktop")]
pub(crate) mod autostart;
#[cfg(feature = "self-update")]
mod download;
pub(crate) mod on_path;
#[cfg(feature = "self-update")]
pub(crate) mod rustic;
#[cfg(feature = "self-update")]
pub(crate) mod update;

/// Whether a package manager keeps `exe` where it is, and updates it there: one that the user runs keeps it
/// in a folder of its own, wherever its root is (Homebrew's `Cellar`, `WinGet`'s `Packages`, Scoop's `apps`
/// beside its `shims`, Chocolatey's `lib` beside `choco.exe`); the system's, in a folder that is not the
/// user's, as `/usr/bin` or `/nix/store`; the macOS app's copy of one of those. A copy elsewhere is the user's.
#[cfg(any(feature = "desktop", feature = "self-update"))]
#[expect(clippy::disallowed_methods, reason = "looks for package managers' folders")]
pub(crate) fn managed(exe: &std::path::Path) -> bool {
    use std::path::Path;
    // The macOS app's copy of one.
    #[cfg(all(target_vendor = "apple", feature = "desktop"))]
    if autostart::app_source(exe).is_some() {
        return true;
    }
    // Where a link leads, as Homebrew's `bin` to its `Cellar`: macOS names the executable as it was started.
    let exe = &exe.canonicalize().unwrap_or_else(|_| exe.to_path_buf());
    let named = |dir: &Path, name: &str| dir.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name));
    let kept = exe.ancestors().any(|dir| {
        named(dir, "Cellar")
            || (named(dir, "Packages") && dir.parent().is_some_and(|p| named(p, "WinGet")))
            || (named(dir, "apps") && dir.parent().is_some_and(|p| p.join("shims").is_dir()))
            || (named(dir, "lib") && dir.parent().is_some_and(|p| p.join("choco.exe").is_file()))
    });
    kept || !exe.parent().is_some_and(crate::sys::writable)
}

#[cfg(all(test, any(feature = "desktop", feature = "self-update")))]
#[expect(clippy::disallowed_methods, reason = "lays out package managers' folders")]
mod tests {
    use std::path::Path;

    use super::managed;
    use crate::sys::writable;

    /// The app copies Homebrew's executable, and names it in its resources.
    #[cfg(all(target_vendor = "apple", feature = "desktop"))]
    #[test]
    fn the_macos_apps_copy_of_homebrews_executable_is_managed() {
        let app = std::env::temp_dir().join(format!("{}-app-{}.app", crate::NAME, std::process::id()));
        let exe = app.join("Contents/MacOS").join(crate::APP_NAME);
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::create_dir_all(app.join("Contents/Resources")).unwrap();
        std::fs::write(&exe, b"").unwrap();
        let mine = managed(&exe);
        std::fs::write(app.join("Contents/Resources/source"), "/opt/homebrew/bin/rusticlone\n").unwrap();
        let source = super::autostart::app_source(&exe);
        let found = managed(&exe);
        std::fs::remove_dir_all(&app).unwrap();
        assert!(!mine && found);
        assert_eq!(source, Some(Path::new("/opt/homebrew/bin/rusticlone").to_path_buf()));
    }

    #[test]
    fn a_copy_in_the_users_folder_is_not_managed() {
        assert!(!managed(&std::env::temp_dir().join(".local").join("bin").join(crate::NAME)));
    }

    /// Started through Homebrew's `bin`, macOS names the executable by the link.
    #[cfg(unix)]
    #[test]
    fn homebrews_cellar_is_managed_through_its_link() {
        let root = std::env::temp_dir().join(format!("{}-brew-{}", crate::NAME, std::process::id()));
        let (cellar, bin) = (root.join("Cellar/rusticlone/2.0.1/bin"), root.join("bin"));
        std::fs::create_dir_all(&cellar).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(cellar.join(crate::NAME), b"").unwrap();
        std::os::unix::fs::symlink(cellar.join(crate::NAME), bin.join(crate::NAME)).unwrap();
        let found = managed(&bin.join(crate::NAME));
        std::fs::remove_dir_all(&root).unwrap();
        assert!(found);
    }

    #[cfg(unix)]
    #[test]
    fn homebrews_cellar_is_managed() {
        for prefix in ["/opt/homebrew", "/usr/local", "/home/linuxbrew/.linuxbrew"] {
            assert!(managed(&Path::new(prefix).join("Cellar/rusticlone/2.0.1/bin/rusticlone")), "{prefix}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn wingets_packages_are_managed() {
        let user = r"C:\Users\me\AppData\Local\Microsoft\WinGet\Packages\AlphaJack.Rusticlone_Source\rusticlone.exe";
        let machine = r"C:\Program Files\WinGet\Packages\AlphaJack.Rusticlone_Source\rusticlone.exe";
        assert!(managed(Path::new(user)) && managed(Path::new(machine)));
    }

    /// Found by its layout, as `ChocolateyInstall` may move its root anywhere.
    #[cfg(windows)]
    #[test]
    fn chocolateys_lib_is_managed_wherever_its_root_is() {
        let root = std::env::temp_dir().join(format!("{}-chocolatey-{}", crate::NAME, std::process::id()));
        let exe = root.join(r"lib\rusticlone\tools\rusticlone.exe");
        std::fs::create_dir_all(&root).unwrap();
        assert!(!managed(&exe));
        std::fs::write(root.join("choco.exe"), b"").unwrap();
        let found = managed(&exe);
        std::fs::remove_dir_all(&root).unwrap();
        assert!(found);
    }

    /// Found by its layout, as `SCOOP` may move its root anywhere.
    #[cfg(windows)]
    #[test]
    fn scoops_apps_are_managed_wherever_its_root_is() {
        let root = std::env::temp_dir().join(format!("{}-scoop-{}", crate::NAME, std::process::id()));
        let exe = root.join(r"apps\rusticlone\current\rusticlone.exe");
        assert!(!managed(&exe));
        std::fs::create_dir_all(root.join("shims")).unwrap();
        let found = managed(&exe);
        std::fs::remove_dir_all(&root).unwrap();
        assert!(found);
    }

    #[test]
    fn the_temporary_folder_is_replaceable() {
        assert!(writable(&std::env::temp_dir()));
    }

    /// As `/usr/bin` of a package. Root writes there anyway.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_folder_only_root_writes_is_not_replaceable_on_linux() {
        if crate::sys::is_root() {
            return;
        }
        assert!(!writable(Path::new("/usr/bin")));
    }

    /// The system volume is read-only, even for root.
    #[cfg(target_vendor = "apple")]
    #[test]
    fn the_system_volume_is_not_replaceable_on_macos() {
        assert!(!writable(Path::new("/usr/bin")));
    }

    /// Windows is not asked: its own folders are writable by whom they belong to.
    #[cfg(windows)]
    #[test]
    fn any_folder_is_replaceable_on_windows() {
        assert!(writable(Path::new(r"C:\Windows")));
    }
}
