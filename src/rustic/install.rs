//! Rustic's executable and its version.

#![expect(clippy::disallowed_methods, reason = "finds rustic's executable")]

use std::{env, path::PathBuf, sync::LazyLock};

use anyhow::Result;
use serde::Serialize;

use super::Version;
use crate::settings;

pub(crate) fn name() -> String {
    format!("rustic{}", env::consts::EXE_SUFFIX)
}

/// Where a downloaded rustic is kept: beside rusticlone, on `PATH`, so that the user can run it too.
pub(crate) fn downloaded() -> Result<PathBuf> {
    Ok(crate::sys::bin_dir()?.join(name()))
}

/// [`settings::RUSTIC`], else the rustic in `PATH`, else the downloaded one, which a service started
/// without the user's `PATH` does not find there. Found at each call, because it may have been installed
/// since.
pub(crate) fn bin() -> PathBuf {
    if let Some(path) = settings::RUSTIC.get(&[]) {
        return path.into();
    }
    let in_path = env::var_os("PATH").and_then(|p| env::split_paths(&p).map(|d| d.join(name())).find(|f| f.is_file()));
    in_path.or_else(|| downloaded().ok().filter(|d| d.is_file())).unwrap_or_else(|| name().into())
}

/// The release the tests run, from `Cargo.toml`, which `the_tested_release_is_known` checks.
pub(crate) fn pinned() -> Version {
    let manifest: Option<toml_edit::DocumentMut> = include_str!("../../Cargo.toml").parse().ok();
    let pin = manifest.as_ref().and_then(|m| m["workspace"]["metadata"]["rustic"]["version"].as_str());
    pin.and_then(|v| Version::parse(v.trim_start_matches('v')).ok()).unwrap_or(Version::new(0, 0, 0))
}

/// The rustic release whose output is parsed: the newest in `tests/fixtures` (`cargo xtask
/// fixtures`), and the one CI tests with and that is downloaded, pinned in `Cargo.toml`. Older releases
/// are refused. Newer ones are parsed the same way, with a warning. When a release changes its output, parsers branch on [`super::Rustic::version`].
pub(crate) static TESTED: LazyLock<Version> = LazyLock::new(pinned);

/// How to install rustic by hand.
pub(crate) const DOCS: &str = "https://rustic.cli.rs/docs/installation.html";

/// A rustic newer than [`TESTED`], whose output may be parsed wrongly. It concerns every collection, so it
/// is shown once, and no collection's level counts it.
#[derive(Debug, Clone, Serialize, crate::i18n::Fluent)]
#[serde(tag = "kind", rename = "rustic-untested")]
#[fluent(prefix = "rustic-untested", tested = format!("{}.{}", TESTED.major, TESTED.minor))]
pub(crate) struct Untested {
    pub found: Version,
}

crate::log::level!(Warning: Untested);

/// From `rustic --version`: `rustic 0.11.4`, `rustic v0.12.0-dev`.
pub(crate) fn parse_version(text: &str) -> Option<Version> {
    Version::parse(text.split_whitespace().last()?.trim_start_matches('v')).ok()
}

pub(super) fn minor(version: &Version) -> (u64, u64) {
    (version.major, version.minor)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    #[expect(clippy::disallowed_methods, reason = "checks the recorded fixtures")]
    fn versions() {
        assert_eq!(parse_version("rustic 0.11.4\n"), Some(Version::new(0, 11, 4)));
        assert!(parse_version("rustic v0.12.0-dev").is_some_and(|v| minor(&v) == (0, 12)));
        assert_eq!(parse_version("rustic"), None);
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        assert!(fixtures.join(format!("rustic-{}.{}", TESTED.major, TESTED.minor)).is_dir(), "cargo xtask fixtures");
    }

    #[test]
    fn the_tested_release_is_known() {
        assert_ne!(pinned(), Version::new(0, 0, 0));
    }
}
