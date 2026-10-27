//! Where `install` keeps the executable: where the installer script puts it, on `PATH`, so that the server
//! does not run a file left in a folder such as Downloads, and new terminals find the command.

#![expect(clippy::disallowed_methods, reason = "copies the executable")]

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

/// The executable to install: this one if its folder is on `PATH`, as for a package or after the installer
/// script, else one in [`sys::bin_dir`].
pub(super) fn target() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    if exe.parent().is_some_and(on_path::has) {
        return Ok(exe);
    }
    Ok(sys::bin_dir()?.join(exe.file_name().unwrap_or_default()))
}

/// Copies this executable to `placed` unless it is that one, and puts its folder on `PATH` for new terminals.
pub(super) fn place(placed: &Path) -> Result<()> {
    let (exe, dir) = (std::env::current_exe()?, placed.parent().unwrap_or(placed));
    if placed != exe {
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
