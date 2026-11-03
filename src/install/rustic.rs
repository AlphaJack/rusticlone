//! Rustic's release, downloaded when rustic is missing or the downloaded one is not the release
//! rusticlone is tested with. Rusticlone connects nowhere unless the user agrees in a terminal, or
//! installed the server with `install --auto-update`: then it downloads that release, checks its
//! signature with the key of rustic's maintainers, and puts it beside rusticlone, on `PATH` for new terminals.

use anyhow::Result;

use super::download::{fetch, install, untar};
use crate::{
    i18n::t,
    rustic::{Cli, Version, bin, downloaded, name, parse_version, pinned},
};

/// The key that signs rustic's releases (`.sig`), from <https://rustic.cli.rs/docs/nightly_builds.html>.
const KEY: &str = "RWSWSCEJEEacVeCy0va71hlrVtiW8YzMzOyJeso0Bfy/ZXq5OryWi/8T";
const RELEASES: &str = "https://github.com/rustic-rs/rustic/releases/download";

/// Rustic's release for this system, as named in its assets. Linux takes the static builds where they exist.
fn target() -> Option<&'static str> {
    target_of(std::env::consts::OS, std::env::consts::ARCH)
}

/// [`target`] for the system `os` on `arch`, as `std::env::consts` names them.
fn target_of(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-musl"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-musl"),
        ("linux", "arm") => Some("armv7-unknown-linux-gnueabihf"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "release-error")]
pub(crate) enum Error {
    NoRelease,
    Unsigned { archive: String, source: minisign_verify::Error },
}

/// What the terminal asks.
#[derive(crate::i18n::Fluent)]
#[fluent(prefix = "release-ask")]
enum Ask {
    #[fluent(releases = RELEASES)]
    Download { version: String, dir: String },
    #[fluent(releases = RELEASES)]
    Replace { found: String, version: String, dir: String },
}

/// When rustic cannot be found, downloads it; when the one where it would download is older than the
/// pinned release, replaces it. One the user installed there, with `cargo install rustic-rs`, is never
/// downgraded. `ask`: only if the terminal agrees; else without asking, as `install --auto-update` and
/// `serve --auto-update` do.
#[cfg_attr(test, mutants::skip)] // Downloads rustic from GitHub.
pub(crate) fn download(ask: bool) -> Result<()> {
    let (path, version) = (downloaded()?, pinned());
    let dir = path.parent().unwrap_or(&path).display().to_string();
    let here = bin::path().canonicalize().ok().zip(path.canonicalize().ok()).is_some_and(|(a, b)| a == b);
    let ran = Cli::rustic().arg("--version").output().map(|out| parse_version(&String::from_utf8_lossy(&out.stdout)));
    let question = match next(ran, here, &version, dir) {
        Next::Ask(question) => question,
        Next::Found(shown) => {
            crate::log::did(&t!("release-found", version = shown, path = bin::path().display().to_string()));
            return Ok(());
        }
        Next::Nothing => return Ok(()),
    };
    let Some(target) = target() else { return Err(Error::NoRelease.into()) };
    if !agreed(ask, &question, &version, crate::log::ask)? {
        crate::log::did(&t!("release-declined"));
        return Ok(());
    }
    crate::log::now(&t!("release-downloading", version = version.to_string()));
    let archive = format!("{RELEASES}/v{version}/rustic-v{version}-{target}.tar.gz");
    let (data, signature) = (fetch(&archive)?, fetch(&format!("{archive}.sig"))?);
    let key = minisign_verify::PublicKey::from_base64(KEY)?;
    let signature = minisign_verify::Signature::decode(&String::from_utf8_lossy(&signature))?;
    key.verify(&data, &signature, false).map_err(|source| Error::Unsigned { archive: archive.clone(), source })?;
    install(&untar(&archive, &data, &name())?, &path)?;
    crate::log::did(&t!("release-downloaded", version = version.to_string(), path = path.display().to_string()));
    super::path::put(path.parent().unwrap_or(&path))
}

/// What [`download`] does about the rustic that runs.
enum Next {
    Ask(Ask),
    /// It runs, as this version shows it.
    Found(String),
    /// It cannot be asked its version, for another reason than not being there.
    Nothing,
}

/// What to do about the rustic that ran, with what `--version` said: download `version` into `dir` when none runs,
/// and replace one there, `here`, that is older. One elsewhere, or one as new, is left alone.
fn next(ran: std::io::Result<Option<Version>>, here: bool, version: &Version, dir: String) -> Next {
    match ran {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Next::Ask(Ask::Download { version: version.to_string(), dir })
        }
        Err(_) => Next::Nothing,
        Ok(found) => {
            let shown = found.as_ref().map(ToString::to_string).unwrap_or_default();
            if !here || found.is_some_and(|f| f >= *version) {
                return Next::Found(shown);
            }
            Next::Ask(Ask::Replace { found: shown, version: version.to_string(), dir })
        }
    }
}

/// Whether to go on with `question`: at once unless `ask`, else as `confirm` answers, given the question and its
/// short form for a command's step.
fn agreed(
    ask: bool,
    question: &Ask,
    version: &Version,
    confirm: impl FnOnce(&str, &str) -> std::io::Result<bool>,
) -> std::io::Result<bool> {
    if !ask {
        return Ok(true);
    }
    let short = match question {
        Ask::Download { .. } => t!("release-short-download", version = version.to_string()),
        Ask::Replace { found, .. } => t!("release-short-replace", found = found.clone(), version = version.to_string()),
    };
    confirm(&question.to_string(), &short)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each supported system and architecture has rustic's release built for it, and others have none.
    #[test]
    fn each_system_has_its_own_release() {
        let systems = [
            ("linux", "x86_64"),
            ("linux", "aarch64"),
            ("linux", "arm"),
            ("macos", "aarch64"),
            ("macos", "x86_64"),
            ("windows", "x86_64"),
        ];
        for (os, arch) in systems {
            let target = target_of(os, arch).unwrap_or_else(|| panic!("{os} on {arch}"));
            let family = match os {
                "linux" => "-linux-",
                "macos" => "-apple-darwin",
                _ => "-windows-",
            };
            assert!(
                target.starts_with(arch.replace("arm", "armv7").as_str()) && target.contains(family),
                "{os} on {arch}: {target}"
            );
        }
        assert_eq!(target_of("freebsd", "x86_64"), None);
    }

    /// Rustic is downloaded where none runs, and replaces an older one in the download folder. One elsewhere, as
    /// new, or that cannot be asked, is left alone.
    #[test]
    fn rustic_is_downloaded_where_none_or_an_older_one_runs() {
        let (pinned, older, newer) = (Version::new(0, 11, 4), Version::new(0, 10, 0), Version::new(0, 12, 0));
        let next = |ran: std::io::Result<Option<Version>>, here| next(ran, here, &pinned, "dir".into());
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert!(matches!(next(Err(missing), false), Next::Ask(Ask::Download { .. })));
        assert!(matches!(next(Err(std::io::Error::other("busy")), true), Next::Nothing));
        assert!(
            matches!(next(Ok(Some(older.clone())), true), Next::Ask(Ask::Replace { found, .. }) if found == "0.10.0")
        );
        assert!(matches!(next(Ok(None), true), Next::Ask(Ask::Replace { .. })), "its version is unknown");
        assert!(matches!(next(Ok(Some(older)), false), Next::Found(found) if found == "0.10.0"), "installed elsewhere");
        assert!(matches!(next(Ok(Some(pinned.clone())), true), Next::Found(_)));
        assert!(matches!(next(Ok(Some(newer)), true), Next::Found(_)), "never downgraded");
    }

    /// Without asking, the download goes on. Asked, it goes on as the answer says, to the question and its short form.
    #[test]
    fn a_download_goes_on_as_agreed() {
        let (version, question) =
            (Version::new(0, 11, 4), Ask::Download { version: "0.11.4".into(), dir: "dir".into() });
        assert!(agreed(false, &question, &version, |_, _| panic!("not asked")).unwrap());
        for answer in [true, false] {
            let said = agreed(true, &question, &version, |long, short| {
                assert!(long.contains("0.11.4") && short.contains("0.11.4") && long != short);
                Ok(answer)
            });
            assert_eq!(said.unwrap(), answer);
        }
    }

    #[test]
    fn this_system_has_a_signed_release() {
        assert!(target().is_some());
        assert!(minisign_verify::PublicKey::from_base64(KEY).is_ok());
    }
}
