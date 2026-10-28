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

/// Who keeps an executable where it is, and updates it there: shown as how to update it with that.
#[cfg(any(feature = "desktop", feature = "self-update"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, crate::i18n::Fluent)]
#[fluent(prefix = "update-with")]
pub(crate) enum Keeper {
    Homebrew,
    Scoop,
    Chocolatey,
    #[fluent(id = format!("{}.{}", owner(), crate::APP_NAME))]
    WinGet,
    Nix,
    /// A container, whose image is pulled again.
    #[cfg_attr(not(target_os = "linux"), expect(dead_code, reason = "containers run Linux"))]
    #[fluent(image = image())]
    Docker,
    #[cfg_attr(not(target_os = "linux"), expect(dead_code, reason = "containers run Linux"))]
    #[fluent(image = image())]
    Podman,
    /// A folder that is not the user's, or the system packages' even for root: `/usr/bin`, `/bin`, `/usr/sbin`.
    System,
}

/// The repository's owner on GitHub, who publishes the `WinGet` package and the container image.
#[cfg(any(feature = "desktop", feature = "self-update"))]
fn owner() -> &'static str {
    env!("CARGO_PKG_REPOSITORY").trim_start_matches("https://github.com/").split('/').next().unwrap_or_default()
}

/// The container image, at its newest release.
#[cfg(any(feature = "desktop", feature = "self-update"))]
fn image() -> String {
    format!("ghcr.io/{}/{}:latest", owner().to_lowercase(), crate::NAME)
}

/// Whether a package manager keeps `exe` where it is: see [`keeper`].
#[cfg(any(feature = "desktop", feature = "self-update"))]
pub(crate) fn managed(exe: &std::path::Path) -> bool {
    keeper(exe).is_some()
}

/// Who keeps `exe` where it is: a package manager that the user runs keeps it in a folder of its own, wherever
/// its root is (Homebrew's `Cellar`, `WinGet`'s `Packages`, Scoop's `apps` beside its `shims`, Chocolatey's
/// `lib` beside `choco.exe`); Nix in its read-only store; the system in its packages' folders and in any that
/// is not the user's; a container's image any. The macOS app's copy of one is kept by the same. `None` for the
/// user's own copy.
#[cfg(any(feature = "desktop", feature = "self-update"))]
#[expect(clippy::disallowed_methods, reason = "looks for package managers' folders")]
pub(crate) fn keeper(exe: &std::path::Path) -> Option<Keeper> {
    use std::path::Path;
    #[cfg(target_os = "linux")]
    // The file that each puts in its containers.
    for (marker, keeper) in [("/.dockerenv", Keeper::Docker), ("/run/.containerenv", Keeper::Podman)] {
        if Path::new(marker).exists() {
            return Some(keeper);
        }
    }
    #[cfg(all(target_vendor = "apple", feature = "desktop"))]
    if let Some(source) = autostart::app_source(exe) {
        return Some(keeper(&source).unwrap_or(Keeper::Homebrew));
    }
    // Where a link leads, as Homebrew's `bin` to its `Cellar`: macOS names the executable as it was started.
    let exe = &exe.canonicalize().unwrap_or_else(|_| exe.to_path_buf());
    let named = |dir: &Path, name: &str| dir.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name));
    let kept = exe.ancestors().find_map(|dir| {
        let parent = dir.parent();
        if named(dir, "Cellar") {
            Some(Keeper::Homebrew)
        } else if named(dir, "Packages") && parent.is_some_and(|p| named(p, "WinGet")) {
            Some(Keeper::WinGet)
        } else if named(dir, "apps") && parent.is_some_and(|p| p.join("shims").is_dir()) {
            Some(Keeper::Scoop)
        } else if named(dir, "lib") && parent.is_some_and(|p| p.join("choco.exe").is_file()) {
            Some(Keeper::Chocolatey)
        } else if named(dir, "store") && parent.is_some_and(|p| named(p, "nix") && p.parent() == Some(Path::new("/"))) {
            Some(Keeper::Nix)
        } else {
            None
        }
    });
    // Packages' folders, which root can write too: a copy of root's own goes in `/usr/local/bin`.
    let system = |dir: &Path| ["/usr/bin", "/bin", "/usr/sbin"].iter().any(|d| dir == Path::new(d));
    let kept = kept.or_else(|| exe.parent().filter(|dir| system(dir)).map(|_| Keeper::System));
    kept.or_else(|| (!exe.parent().is_some_and(crate::sys::writable)).then_some(Keeper::System))
}

#[cfg(all(test, any(feature = "desktop", feature = "self-update")))]
#[expect(clippy::disallowed_methods, reason = "lays out package managers' folders")]
mod tests {
    use std::path::Path;

    use super::{Keeper, keeper, managed};
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
        std::fs::write(app.join("Contents/Resources/source"), "/opt/homebrew/bin/rusticlone\nabc123\n").unwrap();
        let source = super::autostart::app_source(&exe);
        let found = managed(&exe);
        std::fs::remove_dir_all(&app).unwrap();
        assert!(!mine && found);
        assert_eq!(source, Some(Path::new("/opt/homebrew/bin/rusticlone").to_path_buf()));
    }

    /// Root can write them, but a package put the executable there.
    #[cfg(unix)]
    #[test]
    fn the_system_packages_folders_are_managed_for_root_too() {
        for dir in ["/usr/bin", "/bin", "/usr/sbin"] {
            assert_eq!(keeper(&Path::new(dir).join(crate::NAME)), Some(Keeper::System), "{dir}");
        }
    }

    /// Each says its own command.
    #[test]
    fn each_keeper_names_how_to_update_with_it() {
        assert!(Keeper::WinGet.to_string().contains("winget upgrade AlphaJack.Rusticlone"), "{}", Keeper::WinGet);
        assert!(Keeper::Homebrew.to_string().contains("brew upgrade rusticlone"), "{}", Keeper::Homebrew);
        for keeper in [Keeper::Scoop, Keeper::Chocolatey, Keeper::Nix, Keeper::System] {
            assert!(!keeper.to_string().is_empty(), "{keeper:?}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_container_names_its_tool_and_image() {
        let podman = Keeper::Podman.to_string();
        assert!(podman.contains("podman pull ghcr.io/alphajack/rusticlone:latest"), "{podman}");
        assert!(Keeper::Docker.to_string().contains("docker pull"), "{}", Keeper::Docker);
    }

    #[cfg(unix)]
    #[test]
    fn nixs_store_is_managed() {
        assert_eq!(keeper(Path::new("/nix/store/abc-rusticlone-2.0.1/bin/rusticlone")), Some(Keeper::Nix));
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
            let exe = Path::new(prefix).join("Cellar/rusticlone/2.0.1/bin/rusticlone");
            assert_eq!(keeper(&exe), Some(Keeper::Homebrew), "{prefix}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn wingets_packages_are_managed() {
        let user = r"C:\Users\me\AppData\Local\Microsoft\WinGet\Packages\AlphaJack.Rusticlone_Source\rusticlone.exe";
        let machine = r"C:\Program Files\WinGet\Packages\AlphaJack.Rusticlone_Source\rusticlone.exe";
        assert_eq!((keeper(Path::new(user)), keeper(Path::new(machine))), (Some(Keeper::WinGet), Some(Keeper::WinGet)));
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
        let found = keeper(&exe);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(found, Some(Keeper::Chocolatey));
    }

    /// Found by its layout, as `SCOOP` may move its root anywhere.
    #[cfg(windows)]
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
