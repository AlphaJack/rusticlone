//! Where rusticlone and its configuration go, and paths written for another system.

use std::path::PathBuf;

use super::{is_root, os};

/// `app`'s configuration directories, the user's then the system's, laid out as the `directories` crate
/// that rustic uses lays them out.
pub(crate) fn config_dirs(app: &str) -> Result<Vec<PathBuf>, etcetera::HomeDirError> {
    let base = etcetera::base_strategy::choose_native_strategy()?;
    Ok(std::iter::once(os::user_config_dir(&base, app)).chain(os::system_config_dir(app)).collect())
}

/// Where `install` places rusticlone, on `PATH`, unless it runs from [`installer_dir`] already: that folder,
/// or for root or an administrator, whose server is the system's, a system folder. A downloaded rustic goes
/// there too.
pub(crate) fn bin_dir() -> Result<PathBuf, etcetera::HomeDirError> {
    if is_root() {
        return Ok(os::system_bin_dir());
    }
    installer_dir()
}

/// Where the installer script puts rusticlone, for root too: `$CARGO_HOME/bin`, else `~/.cargo/bin`.
#[expect(clippy::disallowed_methods, reason = "the installer script's folder is in the environment")]
pub(crate) fn installer_dir() -> Result<PathBuf, etcetera::HomeDirError> {
    let cargo = std::env::var_os("CARGO_HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    Ok(cargo.map_or_else(|| etcetera::home_dir().map(|h| h.join(".cargo")), Ok)?.join("bin"))
}

/// Where `app`'s configuration is written: the system's directory for root, else the user's.
pub(crate) fn own_config_dir(app: &str) -> Option<PathBuf> {
    config_dirs(app).ok()?.into_iter().nth(usize::from(is_root()))
}

/// An empty directory for test `name`, below one directory per test process in the temporary
/// directory. Directories of ended processes are removed.
#[cfg(test)]
#[expect(clippy::disallowed_methods, reason = "creates test directories")]
pub(crate) fn test_dir(name: &str) -> PathBuf {
    static ENDED_GONE: std::sync::Once = std::sync::Once::new();
    let root = std::env::temp_dir().join(crate::NAME);
    ENDED_GONE.call_once(|| {
        for run in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let pid = run.file_name().to_string_lossy().into_owned();
            if pid.parse::<u32>().is_ok() && os::ended(&pid) {
                _ = std::fs::remove_dir_all(run.path());
            }
        }
    });
    let dir = root.join(std::process::id().to_string()).join(name);
    _ = std::fs::remove_dir_all(&dir);
    _ = std::fs::create_dir_all(&dir);
    dir
}

#[cfg(test)]
mod tests {
    use crate::sys::foreign_path;

    /// A relative folder is this system's, on any system.
    #[test]
    fn relative_folders_are_not_foreign() {
        assert!(!foreign_path("C:relative") && !foreign_path(""));
    }
}
