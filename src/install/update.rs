//! Replaces the running executable with the one of the newest release, built by dist for this system,
//! once its checksum and GitHub's attestation that the release workflow built it both hold.

#![expect(clippy::disallowed_methods, reason = "writes the new executable beside the temporary files")]

use std::{collections::BTreeMap, env::consts::EXE_SUFFIX, path::Path};

use anyhow::Result;
use semver::Version;
use serde::{Deserialize, Serialize};

#[cfg(not(windows))]
use super::download::untar as unpack;
#[cfg(windows)]
use super::download::unzip as unpack;
use super::download::{fetch, install};

use crate::log::{Level, Message};

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
/// What dist uploads with each release: the artifacts it built, for which targets, and their checksums.
const MANIFEST: &str = "dist-manifest.json";

#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "update-error")]
enum Error {
    Checksum {
        file: String,
    },
    #[fluent(manifest = MANIFEST)]
    NoManifest {
        tag: String,
    },
    #[fluent(target = env!("TARGET"))]
    NoArchive {
        tag: String,
    },
}

#[derive(Debug, Serialize, crate::i18n::Fluent)]
#[fluent(prefix = "update")]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum Logged {
    Current {
        current: String,
    },
    Available {
        from: String,
        to: String,
    },
    Updated {
        from: String,
        to: String,
    },
    /// Not downloaded: a package manager updates this executable, or its folder is not the user's.
    Packaged {
        from: String,
        to: String,
    },
    /// At start, which goes on with the executable and rustic as they are.
    #[cfg_attr(not(feature = "server"), expect(dead_code, reason = "only the server updates at start"))]
    NotUpdated {
        error: String,
    },
}

impl Message for Logged {
    fn level(&self) -> Level {
        match self {
            Self::NotUpdated { .. } | Self::Packaged { .. } => Level::Warning,
            Self::Current { .. } | Self::Available { .. } | Self::Updated { .. } => Level::Success,
        }
    }
}

/// The parts of GitHub's release that are read.
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// The parts of dist's manifest that are read.
#[derive(Deserialize)]
struct Manifest {
    artifacts: BTreeMap<String, Artifact>,
}

#[derive(Deserialize)]
struct Artifact {
    kind: String,
    #[serde(default)]
    target_triples: Vec<String>,
    #[serde(default)]
    assets: Vec<Packed>,
    #[serde(default)]
    checksums: BTreeMap<String, String>,
}

/// A file in an archive.
#[derive(Deserialize)]
struct Packed {
    kind: String,
    path: Option<String>,
}

/// An archive with the executable for `target`: its name, the executable's path in it, and its SHA-256. Without
/// one for `target` itself, one for the same processor, vendor and system, which runs as well: a build for
/// Windows's GNU toolchain takes dist's MSVC one, a build for glibc dist's static musl one.
fn archive<'m>(manifest: &'m Manifest, target: &str) -> Option<(&'m str, &'m str, &'m str)> {
    let built_for = |matches: &dyn Fn(&str) -> bool| {
        manifest.artifacts.iter().find_map(|(name, a)| {
            if a.kind != "executable-zip" || !a.target_triples.iter().any(|t| matches(t)) {
                return None;
            }
            let executable = a.assets.iter().find(|p| p.kind == "executable")?.path.as_deref()?;
            Some((name.as_str(), executable, a.checksums.get("sha256")?.as_str()))
        })
    };
    built_for(&|t| t == target).or_else(|| built_for(&|t| system(t).eq(system(target))))
}

/// A target's processor, vendor and system, without its toolchain: `x86_64-pc-windows` of `x86_64-pc-windows-msvc`.
fn system(target: &str) -> impl Iterator<Item = &str> {
    const PARTS: usize = 3;
    target.split('-').take(PARTS)
}

/// Updates this executable, then rustic if it was downloaded: the new executable does, since it knows
/// the rustic it is tested with. A dry run only says whether there is a newer release.
pub(crate) fn update(dry_run: bool) -> Result<()> {
    let logged = replace(dry_run)?;
    crate::log::event(None, &logged);
    match logged {
        Logged::Updated { .. } => {
            std::process::Command::new(std::env::current_exe()?).arg("update").status()?;
        }
        Logged::Current { .. } if !dry_run => super::rustic::download(true)?,
        Logged::Current { .. } | Logged::Available { .. } | Logged::Packaged { .. } | Logged::NotUpdated { .. } => {}
    }
    Ok(())
}

/// For `serve --auto-update`: updates this executable without asking, else the rustic it is tested with.
/// Returns whether this executable was replaced: the new one, started in its place, updates rustic. A
/// failure, as without a network, is logged, and the server starts as it is.
#[cfg_attr(not(feature = "server"), expect(dead_code, reason = "only the server updates at start"))]
pub(crate) fn at_start() -> bool {
    let not_updated = |e: anyhow::Error| crate::log::event(None, &Logged::NotUpdated { error: format!("{e:#}") });
    match replace(false) {
        Ok(logged) => {
            crate::log::event(None, &logged);
            if matches!(logged, Logged::Updated { .. }) {
                return true;
            }
        }
        Err(e) => not_updated(e),
    }
    _ = super::rustic::download(false).map_err(not_updated);
    false
}

/// Whether this executable can replace itself, as the new one is written beside it: not when a package
/// manager updates it, nor when its folder is not the user's, as `/usr/bin` of a package.
pub(crate) fn replaceable() -> bool {
    std::env::current_exe().is_ok_and(|exe| !managed(&exe) && exe.parent().is_some_and(crate::sys::writable))
}

/// Whether a package manager that the user runs keeps `exe` where it is: Homebrew in its `Cellar`, WinGet in
/// its `Packages`, Scoop in its `apps` beside its `shims`, wherever its root is. A copy elsewhere is the user's.
fn managed(exe: &Path) -> bool {
    let named = |dir: &Path, name: &str| dir.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name));
    exe.ancestors().any(|dir| {
        named(dir, "Cellar")
            || (named(dir, "Packages") && dir.parent().is_some_and(|p| named(p, "WinGet")))
            || (named(dir, "apps") && dir.parent().is_some_and(|p| p.join("shims").is_dir()))
    })
}

/// Downloads the newest release if it is newer, checks it against its checksum, and replaces this
/// executable with it.
fn replace(dry_run: bool) -> Result<Logged> {
    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
    let api = REPOSITORY.replace("://github.com/", "://api.github.com/repos/");
    let Release { tag_name: tag, assets } = serde_json::from_slice(&fetch(&format!("{api}/releases/latest"))?)?;
    let newest = Version::parse(tag.trim_start_matches('v'))?;
    if newest <= current {
        return Ok(Logged::Current { current: current.to_string() });
    }
    let (from, to) = (current.to_string(), newest.to_string());
    if dry_run {
        return Ok(Logged::Available { from, to });
    }
    if !replaceable() {
        return Ok(Logged::Packaged { from, to });
    }
    let url = |name: &str| assets.iter().find(|a| a.name == name).map(|a| a.browser_download_url.clone());
    let manifest: Manifest =
        serde_json::from_slice(&fetch(&url(MANIFEST).ok_or_else(|| Error::NoManifest { tag: tag.clone() })?)?)?;
    let (name, executable, checksum) =
        archive(&manifest, env!("TARGET")).ok_or_else(|| Error::NoArchive { tag: tag.clone() })?;
    let archive = url(name).ok_or_else(|| Error::NoArchive { tag: tag.clone() })?;
    let data = fetch(&archive)?;
    if crate::sys::sha256(&data) != checksum {
        return Err(Error::Checksum { file: archive }.into());
    }
    super::attest::verify(REPOSITORY, &tag, &archive, &data)?;
    let new = std::env::temp_dir().join(format!("{}-{newest}{EXE_SUFFIX}", crate::NAME));
    let executable = Path::new(executable).file_name().unwrap_or_default().to_string_lossy();
    install(&unpack(&archive, &data, &executable)?, &new)?;
    let replaced = self_replace::self_replace(&new);
    std::fs::remove_file(&new)?;
    replaced?;
    // Else the app shows the old version, and its signature no longer covers its executable.
    #[cfg(all(target_vendor = "apple", feature = "desktop"))]
    crate::install::autostart::rewrite_app(&to)?;
    Ok(Logged::Updated { from, to })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Manifest, archive, managed};
    use crate::sys::writable;

    #[test]
    fn a_copy_in_the_users_folder_is_not_managed() {
        assert!(!managed(&std::env::temp_dir().join(".local").join("bin").join(crate::NAME)));
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

    /// A real manifest, of mercurio's release: its artifacts are listed as rusticlone's are.
    #[test]
    fn the_archive_is_found_by_its_target_in_dists_manifest() {
        let manifest: Manifest =
            serde_json::from_str(include_str!("../../tests/fixtures/dist-manifest-mercurio.json")).unwrap();
        let (name, executable, checksum) = archive(&manifest, "x86_64-pc-windows-msvc").unwrap();
        assert_eq!((name, executable), ("mercurio-cli-x86_64-pc-windows-msvc.zip", "mercurio.exe"));
        assert_eq!(checksum, "27fe2ac63575c88f2b3886e2a8244783627120ac146ddf3055dfb4a36634cf67");
        assert_eq!(archive(&manifest, "armv7-unknown-linux-musleabihf").map(|a| a.1), Some("mercurio"));
        assert!(archive(&manifest, "x86_64-unknown-freebsd").is_none());
        // Builds for another toolchain of the same system take dist's.
        assert_eq!(
            archive(&manifest, "x86_64-pc-windows-gnu").map(|a| a.0),
            Some("mercurio-cli-x86_64-pc-windows-msvc.zip")
        );
        let musl = "mercurio-cli-x86_64-unknown-linux-musl.tar.xz";
        assert_eq!(archive(&manifest, "x86_64-unknown-linux-gnu").map(|a| a.0), Some(musl));
        assert_eq!(archive(&manifest, "armv7-unknown-linux-gnueabihf").map(|a| a.1), Some("mercurio"));
    }
}
