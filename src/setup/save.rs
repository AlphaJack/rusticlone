//! Saving the profiles, refusing folders that would lose data.

use std::path::PathBuf;

use super::{Error, Role, Setup};
use crate::{profile, sys};

impl Setup {
    /// Refuses a repository folder that a source holds or that holds a source. Refuses one with other files but no
    /// repository's `config` too. The folder of an existing repository is kept to back up into it again.
    #[expect(clippy::disallowed_methods, reason = "saves the profiles")]
    pub(super) fn check_folders(&self) -> Result<(), Error> {
        let sources = self.sources();
        let mut folders = <Role as strum::IntoEnumIterator>::iter().filter_map(|role| self.folder(role));
        folders.find_map(|folder| refused_folder(&folder, &sources)).map_or(Ok(()), Err)
    }
}

/// Why backups cannot go to `folder`: it is inside one of `sources` or holds one, or it holds other files.
#[cfg(feature = "server")]
pub(crate) fn refused_folder(folder: &str, sources: &[String]) -> Option<Error> {
    use std::path::{Path, PathBuf};
    // As written and as the disk resolves it, up to its nearest folder that exists: `/tmp` is a link to
    // `/private/tmp` on macOS, and `/users/new` is `/Users/new` on a disk that ignores case.
    let resolved = |p: &Path| -> Option<PathBuf> {
        let base = p.ancestors().find_map(|a| Some((a, a.canonicalize().ok()?)))?;
        Some(base.1.join(p.strip_prefix(base.0).ok()?))
    };
    let forms = |p: &str| [Some(Path::new(p).to_path_buf()), resolved(Path::new(p))];
    let nested = |source: &String| {
        forms(folder)
            .iter()
            .flatten()
            .any(|f| forms(source).iter().flatten().any(|s| f.starts_with(s) || s.starts_with(f)))
    };
    if sources.iter().any(nested) {
        return Some(Error::Inside);
    }
    let dir = Path::new(folder);
    let others = std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some());
    (others && !dir.join("config").is_file()).then(|| Error::NotEmpty { folder: folder.to_owned() })
}

/// The folder the form saves to, if this process may write in it.
pub(crate) fn dir() -> Option<PathBuf> {
    profile::own_dir().filter(|d| sys::writable(d))
}

/// Saves the profiles in [`dir`], and returns it. Never overwrites a file.
pub(crate) fn save(setup: &Setup) -> anyhow::Result<PathBuf> {
    save_in(setup, dir())
}

/// [`save`] into `dir`, the folder that [`dir`] finds, if any.
#[expect(clippy::disallowed_methods, reason = "saves the profiles")]
fn save_in(setup: &Setup, dir: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    use std::{fs, io::Write};
    setup.check()?;
    setup.check_folders()?;
    let dir = dir.ok_or(Error::NoDir)?;
    fs::create_dir_all(&dir)?;
    let profiles = setup.profiles();
    if let Some(path) = profiles.iter().map(|(file, _)| dir.join(file)).find(|p| p.exists()) {
        return Err(Error::Exists { path }.into());
    }
    // The target first: if the primary exists, the target left behind backs up nothing.
    for (file, text) in profiles.iter().rev() {
        let path = dir.join(file);
        match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Err(Error::Exists { path }.into()),
            opened => opened?.write_all(text.as_bytes())?,
        }
    }
    Ok(dir)
}
