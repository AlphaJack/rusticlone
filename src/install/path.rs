//! Putting a folder on the user's `PATH` for new terminals, as rustup and the installer script do: for
//! rusticlone, and for the rustic it downloads beside it.

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::log;

#[derive(Debug, Serialize, crate::i18n::Fluent)]
#[fluent(prefix = "install")]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum Logged {
    OnPath { dir: String, changed: String },
}

log::level!(Success: Logged);

/// Whether this process's `PATH` has `dir`.
#[expect(clippy::disallowed_methods, reason = "reads PATH and edits the shells' startup files")]
pub(crate) fn has(dir: &Path) -> bool {
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d == dir))
}

/// Puts `dir` on `PATH` for new terminals, unless this process's has it, and says what changed.
pub(crate) fn put(dir: &Path) -> Result<()> {
    let changed = if has(dir) { String::new() } else { imp::add(dir)? };
    if !changed.is_empty() {
        log::event(None, &Logged::OnPath { dir: dir.display().to_string(), changed });
    }
    Ok(())
}

/// As rustup and the installer script do: an `env` script beside `dir` that puts it on `PATH`, sourced by
/// the shells' startup files.
#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as imp;

/// As the installer script does: prepended to the `Path` that new processes get, the user's or, for an
/// administrator, the system's, and announced to running programs so that new terminals see it.
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

#[cfg(test)]
mod tests {
    /// A folder is on `PATH` as this process has it, not merely because it exists.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "reads this process's PATH, as `has` does")]
    fn a_folder_on_path_is_found() {
        let path = std::env::var_os("PATH").unwrap();
        let first = std::env::split_paths(&path).next().unwrap();
        assert!(super::has(&first));
        assert!(!super::has(&crate::sys::test_dir("not-on-path")));
    }
}
