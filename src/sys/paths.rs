//! Where rusticlone and its configuration go, and paths written for another system.

use std::path::PathBuf;

use etcetera::BaseStrategy;

use super::is_root;

/// `app`'s configuration directories, the user's then the system's, laid out as the `directories` crate
/// that rustic uses lays them out.
#[expect(clippy::disallowed_methods, reason = "the system's directory on Windows is in the environment")]
pub(crate) fn config_dirs(app: &str) -> Result<Vec<PathBuf>, etcetera::HomeDirError> {
    let base = etcetera::base_strategy::choose_native_strategy()?;
    let user = if cfg!(windows) {
        base.config_dir().join(app).join("config")
    } else if cfg!(target_vendor = "apple") {
        base.data_dir().join(app)
    } else {
        base.config_dir().join(app)
    };
    let system = match std::env::var_os("PROGRAMDATA") {
        Some(data) if cfg!(windows) => Some(PathBuf::from(data).join(app).join("config")),
        _ => (!cfg!(windows)).then(|| PathBuf::from("/etc").join(app)),
    };
    Ok([user].into_iter().chain(system).collect())
}

/// Where `install` places rusticlone, on `PATH`, unless it runs from [`installer_dir`] already: that folder,
/// or for root or an administrator, whose server is the system's, a system folder. A downloaded rustic goes
/// there too.
pub(crate) fn bin_dir() -> Result<PathBuf, etcetera::HomeDirError> {
    if is_root() {
        return Ok(super::os::system_bin_dir());
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

/// Whether `path` is an absolute path of a system other than this one: a Windows drive's elsewhere, or a Unix
/// root's on Windows. Its folder has no place here.
pub(crate) fn foreign_path(path: &str) -> bool {
    if cfg!(windows) { path.starts_with('/') } else { windows_drive(path).is_some() }
}

/// The drive letter of a Windows path such as `C:\Users`, and what follows it.
pub(crate) fn windows_drive(path: &str) -> Option<(char, &str)> {
    let mut chars = path.chars();
    let (letter, colon, separator) = (chars.next()?, chars.next()?, chars.next()?);
    let windows = letter.is_ascii_alphabetic() && colon == ':' && (separator == '\\' || separator == '/');
    windows.then(|| (letter, path.get(2..).unwrap_or_default()))
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
            if pid.parse::<u32>().is_ok()
                && !std::path::Path::new("/proc").join(&pid).exists()
                && cfg!(target_os = "linux")
            {
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
    use super::foreign_path;

    /// Another system's absolute folders, which have no place here. Relative ones are this system's.
    #[test]
    fn another_systems_folders_are_told_apart() {
        if cfg!(windows) {
            assert!(foreign_path("/home/me") && !foreign_path(r"C:\Users\me"));
        } else {
            assert!(foreign_path(r"C:\Users\me") && foreign_path("d:/data") && !foreign_path("/home/me"));
        }
        assert!(!foreign_path("C:relative") && !foreign_path(""));
    }
}
