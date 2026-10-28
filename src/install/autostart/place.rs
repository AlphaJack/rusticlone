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

/// The executable to install: this one where it is, if [`kept`], else one in [`sys::bin_dir`].
pub(super) fn target() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let installer = sys::installer_dir().ok();
    match kept(&exe, &std::env::var_os("PATH").unwrap_or_default(), installer.as_deref()) {
        Some(kept) => Ok(kept),
        None => Ok(sys::bin_dir()?.join(exe.file_name().unwrap_or_default())),
    }
}

/// Where `exe` stays, with `path` as `PATH`: a package manager's where it keeps it, named as on `PATH` when a
/// link there leads to it, as Homebrew's to its `Cellar`, since a version's folder goes with the next one;
/// one in a folder on `PATH`; one where the `installer` script put it, for root too, before a new shell has it on
/// `PATH`. `None` for one to copy, as from Downloads.
fn kept(exe: &Path, path: &std::ffi::OsStr, installer: Option<&Path>) -> Option<PathBuf> {
    if crate::install::managed(exe) {
        return Some(linked(exe, path).unwrap_or_else(|| exe.to_path_buf()));
    }
    let on_path = std::env::split_paths(path).any(|dir| exe.parent() == Some(dir.as_path()));
    (on_path || (installer.is_some() && exe.parent() == installer)).then(|| exe.to_path_buf())
}

/// The command in `path`, as `PATH`, that leads to `exe`.
fn linked(exe: &Path, path: &std::ffi::OsStr) -> Option<PathBuf> {
    let (real, name) = (exe.canonicalize().ok()?, exe.file_name()?);
    std::env::split_paths(path).map(|dir| dir.join(name)).find(|link| link.canonicalize().is_ok_and(|l| l == real))
}

/// Copies this executable to `placed` unless that leads to it, and puts its folder on `PATH` for new terminals,
/// unless a package manager keeps it: its own link or shim is on `PATH` already.
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
    if crate::install::managed(placed) {
        return Ok(());
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{kept, linked};

    /// A folder of its own in the temporary one, removed with its contents.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("{}-{name}-{}", crate::NAME, std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        /// An executable at `relative`, with its folders.
        fn exe(&self, relative: &str) -> PathBuf {
            let exe = self.0.join(relative);
            std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
            std::fs::write(&exe, b"").unwrap();
            exe
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn path(dirs: &[&Path]) -> std::ffi::OsString {
        std::env::join_paths(dirs).unwrap()
    }

    #[test]
    fn one_in_a_folder_on_path_stays_and_one_elsewhere_is_copied() {
        let folder = Folder::new("kept");
        let (bin, downloads) = (folder.exe("bin/rusticlone"), folder.exe("Downloads/rusticlone"));
        let on_path = path(&[&folder.0.join("bin")]);
        assert_eq!(kept(&bin, &on_path, None), Some(bin));
        assert_eq!(kept(&downloads, &on_path, None), None);
    }

    /// As root runs the installer script's `~/.cargo/bin/rusticlone install` from the same shell.
    #[test]
    fn one_where_the_installer_script_put_it_stays_before_path_has_it() {
        let folder = Folder::new("installer");
        let (cargo, installer) = (folder.exe(".cargo/bin/rusticlone"), folder.0.join(".cargo/bin"));
        assert_eq!(kept(&cargo, &path(&[&folder.0.join("elsewhere")]), Some(&installer)), Some(cargo.clone()));
        assert_eq!(kept(&cargo, &path(&[&folder.0.join("elsewhere")]), None), None);
    }

    /// As Homebrew's `bin/rusticlone` leads to its `Cellar`: the link outlives the version's folder.
    #[cfg(unix)]
    #[test]
    fn homebrews_executable_stays_named_by_its_link_on_path() {
        let folder = Folder::new("linked");
        let exe = folder.exe("Cellar/rusticlone/2.0.1/bin/rusticlone");
        let bin = folder.0.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(&exe, bin.join(crate::NAME)).unwrap();
        let on_path = path(&[&folder.0.join("elsewhere"), &bin]);
        assert_eq!(linked(&exe, &on_path), Some(bin.join(crate::NAME)));
        assert_eq!(kept(&exe, &on_path, None), Some(bin.join(crate::NAME)));
        // Started through the link, as macOS then names it.
        assert_eq!(kept(&bin.join(crate::NAME), &on_path, None), Some(bin.join(crate::NAME)));
        // Without a link on `PATH`, where the package manager keeps it, rather than a copy.
        assert_eq!(kept(&exe, &path(&[&folder.0.join("elsewhere")]), None), Some(exe));
    }
}
