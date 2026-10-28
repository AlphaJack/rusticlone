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
    /// Not downloaded: a package manager updates this executable, or its folder is not the user's. `how`: the
    /// [`Keeper`](crate::install::Keeper)'s command.
    Packaged {
        from: String,
        to: String,
        how: String,
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
    // Read before it is replaced: Linux then names the old file, with ` (deleted)`.
    let exe = std::env::current_exe()?;
    let logged = replace(dry_run)?;
    crate::log::event(None, &logged);
    match logged {
        Logged::Updated { .. } => {
            std::process::Command::new(exe).arg("update").status()?;
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

/// Whether this executable can replace itself: not when a package manager keeps it.
pub(crate) fn replaceable() -> bool {
    std::env::current_exe().is_ok_and(|exe| !super::managed(&exe))
}

/// What is logged without downloading: this release is the newest, or a dry run, or a package manager keeps
/// this executable, which `how` says how to update. `None` to download the newest.
fn without_download(
    current: &Version,
    newest: &Version,
    dry_run: bool,
    replaceable: bool,
    how: impl FnOnce() -> String,
) -> Option<Logged> {
    let (from, to) = (current.to_string(), newest.to_string());
    if newest <= current {
        Some(Logged::Current { current: from })
    } else if dry_run {
        Some(Logged::Available { from, to })
    } else if !replaceable {
        Some(Logged::Packaged { from, to, how: how() })
    } else {
        None
    }
}

/// Downloads the newest release if it is newer, checks it against its checksum, and replaces this
/// executable with it.
fn replace(dry_run: bool) -> Result<Logged> {
    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
    let api = REPOSITORY.replace("://github.com/", "://api.github.com/repos/");
    let Release { tag_name: tag, assets } = serde_json::from_slice(&fetch(&format!("{api}/releases/latest"))?)?;
    let newest = Version::parse(tag.trim_start_matches('v'))?;
    let how = || {
        let keeper = std::env::current_exe().ok().and_then(|exe| crate::install::keeper(&exe));
        keeper.unwrap_or(crate::install::Keeper::System).to_string()
    };
    if let Some(logged) = without_download(&current, &newest, dry_run, replaceable(), how) {
        return Ok(logged);
    }
    let (from, to) = (current.to_string(), newest.to_string());
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
    use semver::Version;

    use super::{Logged, Manifest, archive, without_download};

    /// A package manager's executable is not downloaded, even with a newer release; the user's is.
    #[test]
    fn a_package_managers_executable_is_left_to_it() {
        let (old, new) = (Version::new(2, 0, 0), Version::new(2, 0, 1));
        let how = || String::from("brew");
        assert!(
            matches!(without_download(&old, &new, false, false, how), Some(Logged::Packaged { how, .. }) if how == "brew")
        );
        assert!(without_download(&old, &new, false, true, how).is_none());
        assert!(matches!(without_download(&old, &new, true, false, how), Some(Logged::Available { .. })));
        assert!(matches!(without_download(&new, &new, false, false, how), Some(Logged::Current { .. })));
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
