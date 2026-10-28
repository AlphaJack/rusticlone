//! Where `install` keeps the executable: where the installer script puts it, on `PATH`, so that the server
//! does not run a file left in a folder such as Downloads, and new terminals find the command. A package
//! manager's stays where it keeps it, so that it updates the one the server runs.

#![expect(clippy::disallowed_methods, reason = "copies the executable and reads PATH")]

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::{install::on_path, log, sys};

#[derive(Debug, Serialize, crate::i18n::Fluent)]
#[fluent(prefix = "place")]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum Logged {
    Copied { from: String, to: String },
}

#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "place-error")]
enum Error {
    Write { path: String },
    Replace { path: String },
}

log::level!(Success: Logged);

/// The executable to install: a package manager's where it keeps it, named as on `PATH` when a link there
/// leads to it, as Homebrew's to its `Cellar`, since a version's folder goes with the next one; this one if
/// its folder is on `PATH`, as after the installer script; else one in [`sys::bin_dir`].
pub(super) fn target() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    if crate::install::managed(&exe) {
        let path = std::env::var_os("PATH").unwrap_or_default();
        return Ok(linked(&exe, &path).unwrap_or(exe));
    }
    if exe.parent().is_some_and(on_path::has) {
        return Ok(exe);
    }
    Ok(sys::bin_dir()?.join(exe.file_name().unwrap_or_default()))
}

/// The command in `path`, as `PATH`, that leads to `exe`.
fn linked(exe: &Path, path: &std::ffi::OsStr) -> Option<PathBuf> {
    let (real, name) = (exe.canonicalize().ok()?, exe.file_name()?);
    std::env::split_paths(path).map(|dir| dir.join(name)).find(|link| link.canonicalize().is_ok_and(|l| l == real))
}

/// Copies this executable to `placed` unless that leads to it, and puts its folder on `PATH` for new terminals.
pub(super) fn place(placed: &Path) -> Result<()> {
    let (exe, dir) = (std::env::current_exe()?, placed.parent().unwrap_or(placed));
    let same = placed.canonicalize().is_ok_and(|p| exe.canonicalize().is_ok_and(|e| e == p));
    if !same {
        std::fs::create_dir_all(dir)?;
        // Replaced whole, so that a copy that runs keeps its file.
        let partial = placed.with_extension("partial");
        std::fs::copy(&exe, &partial).with_context(|| Error::Write { path: partial.display().to_string() })?;
        replace(&partial, placed).with_context(|| Error::Replace { path: placed.display().to_string() })?;
        log::event(None, &Logged::Copied { from: exe.display().to_string(), to: placed.display().to_string() });
    }
    on_path::put(dir)
}

/// Moves `from` over `to`. Windows keeps an executable locked a moment after its process exits, while the
/// system or an antivirus still holds it: retried meanwhile.
fn replace(from: &Path, to: &Path) -> std::io::Result<()> {
    const RETRY: std::time::Duration = std::time::Duration::from_millis(100);
    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        match std::fs::rename(from, to) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && std::time::Instant::now() < deadline => {
                std::thread::sleep(RETRY);
            }
            done => return done,
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    /// As Homebrew's `bin/rusticlone` leads to its `Cellar`.
    #[test]
    fn the_link_on_path_is_found_for_its_target() {
        let root = std::env::temp_dir().join(format!("{}-linked-{}", crate::NAME, std::process::id()));
        let (cellar, bin) = (root.join("Cellar/rusticlone/2.0.1/bin"), root.join("bin"));
        std::fs::create_dir_all(&cellar).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        let exe = cellar.join(crate::NAME);
        std::fs::write(&exe, b"").unwrap();
        std::os::unix::fs::symlink(&exe, bin.join(crate::NAME)).unwrap();
        let path = std::env::join_paths([root.join("elsewhere"), bin.clone()]).unwrap();
        let found = super::linked(&exe, &path);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(found, Some(bin.join(crate::NAME)));
    }
}
