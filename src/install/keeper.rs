//! Who keeps an executable where it is, and updates it there: a package manager, the system or a container.

#[cfg(all(test, target_os = "linux"))]
mod linux;
/// It reads the app, which only the desktop installs.
#[cfg(all(test, feature = "desktop", target_os = "macos"))]
mod macos;
#[cfg(all(test, unix))]
mod unix;
#[cfg(all(test, windows))]
mod windows;

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
pub(crate) fn managed(exe: &std::path::Path) -> bool {
    keeper(exe).is_some()
}

/// Who keeps `exe` where it is: a package manager that the user runs keeps it in a folder of its own, wherever
/// its root is (Homebrew's `Cellar`, `WinGet`'s `Packages`, Scoop's `apps` beside its `shims`, Chocolatey's
/// `lib` beside `choco.exe`); Nix in its read-only store; the system in its packages' folders and in any that
/// is not the user's; a container's image any. The macOS app's copy of one is kept by the same. `None` for the
/// user's own copy.
#[expect(clippy::disallowed_methods, reason = "looks for package managers' folders")]
pub(crate) fn keeper(exe: &std::path::Path) -> Option<Keeper> {
    use std::path::Path;
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
    use super::{Keeper, managed};
    use crate::sys::writable;

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
