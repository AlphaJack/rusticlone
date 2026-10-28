//! Rustic's release, downloaded when rustic is missing or the downloaded one is not the release
//! rusticlone is tested with. Rusticlone connects nowhere unless the user agrees in a terminal, or
//! installed the server with `install --auto-update`: then it downloads that release, checks its
//! signature with the key of rustic's maintainers, and puts it beside rusticlone, on `PATH` for new terminals.

use anyhow::Result;

use super::download::{fetch, install, untar};
use crate::{
    i18n::t,
    rustic::{Cli, bin, downloaded, name, parse_version, pinned},
};

/// The key that signs rustic's releases (`.sig`), from <https://rustic.cli.rs/docs/nightly_builds.html>.
const KEY: &str = "RWSWSCEJEEacVeCy0va71hlrVtiW8YzMzOyJeso0Bfy/ZXq5OryWi/8T";
const RELEASES: &str = "https://github.com/rustic-rs/rustic/releases/download";

/// Rustic's release for this system, as named in its assets. Linux takes the static builds where they exist.
const TARGET: Option<&str> = if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some("x86_64-unknown-linux-musl")
} else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
    Some("aarch64-unknown-linux-musl")
} else if cfg!(all(target_os = "linux", target_arch = "arm")) {
    Some("armv7-unknown-linux-gnueabihf")
} else if cfg!(all(target_vendor = "apple", target_arch = "aarch64")) {
    Some("aarch64-apple-darwin")
} else if cfg!(all(target_vendor = "apple", target_arch = "x86_64")) {
    Some("x86_64-apple-darwin")
} else if cfg!(all(windows, target_arch = "x86_64")) {
    Some("x86_64-pc-windows-msvc")
} else {
    None
};

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
pub(crate) fn download(ask: bool) -> Result<()> {
    let (path, version) = (downloaded()?, pinned());
    let dir = path.parent().unwrap_or(&path).display().to_string();
    let there = |found: std::path::PathBuf| {
        found.canonicalize().ok().zip(path.canonicalize().ok()).is_some_and(|(a, b)| a == b)
    };
    let question = match Cli::rustic().arg("--version").output() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ask::Download { version: version.to_string(), dir },
        Ok(out) => {
            let found = parse_version(&String::from_utf8_lossy(&out.stdout));
            let shown = found.as_ref().map(ToString::to_string).unwrap_or_default();
            if !there(bin()) || found.as_ref().is_some_and(|f| *f >= version) {
                crate::log::did(&t!("release-found", version = shown, path = bin().display().to_string()));
                return Ok(());
            }
            Ask::Replace { found: shown, version: version.to_string(), dir }
        }
        Err(_) => return Ok(()),
    };
    let Some(target) = TARGET else { return Err(Error::NoRelease.into()) };
    let short = match &question {
        Ask::Download { .. } => t!("release-short-download", version = version.to_string()),
        Ask::Replace { found, .. } => t!("release-short-replace", found = found.clone(), version = version.to_string()),
    };
    if ask && !crate::sys::ask(&question.to_string(), &short)? {
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
    super::on_path::put(path.parent().unwrap_or(&path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_system_has_a_signed_release() {
        assert!(TARGET.is_some());
        assert!(minisign_verify::PublicKey::from_base64(KEY).is_ok());
    }
}
