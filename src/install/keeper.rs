//! Who keeps an executable where it is, and updates it there: a package manager, the system or a container.

#[cfg(all(test, target_os = "linux"))]
mod linux;
/// It reads the app, which only the desktop installs.
#[cfg(all(test, feature = "desktop", target_os = "macos"))]
mod macos;
#[cfg(all(test, unix))]
mod unix;

use std::path::Path;

/// Who keeps an executable where it is, and updates it there: shown as how to update it with that.
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
    #[fluent(image = image())]
    Docker,
    #[fluent(image = image())]
    Podman,
    /// A folder that is not the user's, or the system packages' even for root: `/usr/bin`, `/bin`, `/usr/sbin`.
    System,
}

/// The repository's owner on GitHub, who publishes the `WinGet` package and the container image.
fn owner() -> &'static str {
    env!("CARGO_PKG_REPOSITORY").trim_start_matches("https://github.com/").split('/').next().unwrap_or_default()
}

/// The container image, at its newest release.
fn image() -> String {
    format!("ghcr.io/{}/{}:latest", owner().to_lowercase(), crate::NAME)
}

/// Whether a package manager keeps `exe` where it is: see [`keeper`].
pub(crate) fn managed(exe: &Path) -> bool {
    keeper(exe).is_some()
}

/// Who keeps `exe` where it is: a package manager that the user runs keeps it in a folder of its own, wherever
/// its root is (Homebrew's `Cellar`, `WinGet`'s `Packages`, Scoop's `apps` beside its `shims`, Chocolatey's
/// `lib` beside `choco.exe`); Nix in its read-only store; the system in its packages' folders and in any that
/// is not the user's; a container's image any. The macOS app's copy of one is kept by the same. `None` for the
/// user's own copy.
pub(crate) fn keeper(exe: &Path) -> Option<Keeper> {
    match crate::sys::container() {
        Some(crate::sys::Container::Docker) => return Some(Keeper::Docker),
        Some(crate::sys::Container::Podman) => return Some(Keeper::Podman),
        None => {}
    }
    #[cfg(feature = "desktop")]
    if let Some(source) = super::app::app_source(exe) {
        return Some(keeper(&source).unwrap_or(Keeper::Homebrew));
    }
    folders(exe)
}

/// The package manager whose folders hold `exe`, by their names and by what is beside them, or the system's own
/// folders, which a user cannot write.
#[expect(clippy::disallowed_methods, reason = "looks for package managers' folders")]
fn folders(exe: &Path) -> Option<Keeper> {
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
    let system = |dir: &Path| crate::sys::system_bins().iter().any(|d| dir == d);
    let kept = kept.or_else(|| exe.parent().filter(|dir| system(dir)).map(|_| Keeper::System));
    kept.or_else(|| (!exe.parent().is_some_and(crate::sys::writable)).then_some(Keeper::System))
}

#[cfg(test)]
mod tests {
    #![expect(clippy::disallowed_methods, reason = "lays out package managers' folders")]

    use super::{Keeper, Path, folders, managed};
    use crate::sys::writable;

    /// `relative` below a folder of its own in the temporary one, with its folders. The executable itself is not
    /// needed, only where it is.
    fn exe(root: &Path, relative: &[&str]) -> std::path::PathBuf {
        let exe = relative.iter().fold(root.to_owned(), |path, part| path.join(part));
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        exe
    }

    /// `WinGet`'s packages are below its own `Packages`, not any folder of that name.
    #[test]
    fn wingets_packages_are_managed() {
        let root = crate::sys::test_dir("keeper-winget");
        let package = ["Packages", "AlphaJack.Rusticlone_Source", "rusticlone.exe"];
        let winget = exe(&root, &[&["WinGet"][..], &package].concat());
        assert_eq!(folders(&winget), Some(Keeper::WinGet));
        assert_eq!(folders(&exe(&root, &[&["Other"][..], &package].concat())), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Found by its layout, as `ChocolateyInstall` may move its root anywhere.
    #[test]
    fn chocolateys_lib_is_managed_wherever_its_root_is() {
        let root = crate::sys::test_dir("keeper-chocolatey");
        let exe = exe(&root, &["lib", "rusticlone", "tools", "rusticlone.exe"]);
        assert_eq!(folders(&exe), None, "without choco.exe beside it");
        std::fs::write(root.join("choco.exe"), b"").unwrap();
        assert_eq!(folders(&exe), Some(Keeper::Chocolatey));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Found by its layout, as `SCOOP` may move its root anywhere.
    #[test]
    fn scoops_apps_are_managed_wherever_its_root_is() {
        let root = crate::sys::test_dir("keeper-scoop");
        let exe = exe(&root, &["apps", "rusticlone", "current", "rusticlone.exe"]);
        assert_eq!(folders(&exe), None, "without shims beside it");
        std::fs::create_dir_all(root.join("shims")).unwrap();
        assert_eq!(folders(&exe), Some(Keeper::Scoop));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Nix's store is `/nix/store` only, not a folder of that name elsewhere.
    #[test]
    fn a_store_elsewhere_is_not_nixs() {
        let root = crate::sys::test_dir("keeper-nix");
        for at in [&["nix", "store"][..], &["other", "store"]] {
            assert_eq!(folders(&exe(&root, &[at, &["abc-rusticlone", "bin", "rusticlone"]].concat())), None, "{at:?}");
        }
        std::fs::remove_dir_all(root).unwrap();
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

    #[test]
    fn a_copy_in_the_users_folder_is_not_managed() {
        assert!(!managed(&std::env::temp_dir().join(".local").join("bin").join(crate::NAME)));
    }

    #[test]
    fn the_temporary_folder_is_replaceable() {
        assert!(writable(&std::env::temp_dir()));
    }
}
