//! `install`: the server starts with the computer as root or headless on Linux, else at login, through the system's
//! own service manager, and starts now. It runs this executable with the current `PATH`, so that it
//! finds the same rustic.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::log;

#[cfg(any(target_os = "linux", test))]
mod packaging;
mod place;
#[cfg(unix)]
mod unix;

#[derive(Debug, Serialize, crate::i18n::Fluent)]
#[fluent(prefix = "autostart")]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum Logged {
    /// As root, an administrator, the system service, or a headless Linux user.
    Installed {
        file: String,
    },
    InstalledAtLogin {
        file: String,
    },
    Uninstalled {
        file: String,
    },
    /// The server started at login from the `Run` key has no service to stop it: it runs until logout.
    UninstalledRunning {
        file: String,
    },
    #[cfg(feature = "self-update")]
    AutoUpdate {
        file: String,
    },
    /// `install --auto-update` of an executable that its package manager updates.
    #[cfg(feature = "self-update")]
    AutoUpdateManaged,
    NotInstalled,
    Deleted {
        dir: String,
    },
    /// The macOS app copied the executable that the package manager updated.
    #[cfg(feature = "self-update")]
    AppRefreshed {
        version: String,
    },
}

/// The user switched the server off in macOS's Login Items, so launchd no longer starts it.
#[derive(Debug, Serialize, crate::i18n::Fluent)]
#[serde(tag = "kind", rename = "login-item-off")]
#[fluent(prefix = "autostart-blocked", app = crate::APP_NAME)]
pub(crate) struct Blocked {}

log::level!(Success: Logged);
log::level!(Warning: Blocked);

/// Whether the server is installed to start at login, and the user switched it off.
#[cfg_attr(test, mutants::skip)] // Asks the service manager.
pub(crate) fn blocked() -> Option<Blocked> {
    imp::blocked().then_some(Blocked {})
}

/// Starts the installed or packaged server, rather than one without its options, and waits until it answers on
/// `listen`: false without one, or if it does not answer within a minute, which an update at its start can take.
/// The service manager then supervises it, unlike a server that `serve --open` starts.
#[cfg_attr(test, mutants::skip)] // Starts the installed server through the service manager.
pub(crate) fn start_installed(listen: &str) -> Result<bool> {
    Ok(imp::start()? && answers_soon(listen))
}

/// Whether the server answers on `listen` within a minute, which an update at its start can take.
#[cfg_attr(test, mutants::skip)] // Waits up to a minute for a server to answer.
#[expect(clippy::disallowed_methods, reason = "writes the service files and runs the service manager")]
pub(crate) fn answers_soon(listen: &str) -> bool {
    const POLL: std::time::Duration = std::time::Duration::from_millis(200);
    let deadline = std::time::Instant::now() + std::time::Duration::from_mins(1);
    while crate::health::check(listen).is_err() {
        if std::time::Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
    true
}

/// Copies the package manager's executable into the macOS app again when it updated it, and says so.
/// Whether it did: the app then starts again.
#[cfg(feature = "self-update")]
#[cfg_attr(test, mutants::skip)] // Rewrites the installed macOS app.
pub(crate) fn refresh_app() -> Result<bool> {
    let Some(version) = imp::refresh_app()? else { return Ok(false) };
    log::event(None, &Logged::AppRefreshed { version });
    Ok(true)
}

/// Writes the macOS app again around this executable, which an update replaced with `version`: its version,
/// and a signature that covers the new executable. Nothing for an executable outside the app.
#[cfg(feature = "self-update")]
#[cfg_attr(test, mutants::skip)] // Rewrites the installed macOS app.
pub(crate) fn rewrite_app(version: &str) -> Result<()> {
    imp::rewrite_app(version)
}

/// Whether this is the macOS app opened from Finder or Spotlight, which passes no arguments.
#[cfg_attr(test, mutants::skip)] // Asks how macOS started this process.
pub(crate) fn opened_as_app() -> bool {
    imp::opened_as_app()
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;

/// From an elevated shell, a system service that starts with the computer, restarted after a failure; else
/// the user's `Run` key, so that the server starts in the user's session at login, reads the user's profiles,
/// and shows its tray icon and desktop notifications, and a Start menu entry that opens the web UI.
#[cfg(windows)]
pub(crate) mod windows;
#[cfg(windows)]
use windows as imp;

/// The arguments of the installed server: its settings are in the env file, so that a server started
/// another way runs the same.
pub(crate) fn serve_args() -> Vec<String> {
    vec!["serve".into()]
}

/// Puts this executable where the server runs it, and returns where. Installing again updates it and `PATH`.
pub(crate) fn place() -> Result<PathBuf> {
    let exe = place::target()?;
    // Windows keeps a running executable from being replaced. Stopped rather than renamed aside, which
    // Windows allows: Defender quarantined an executable renamed aside while it ran and replaced, with the
    // Run key and the Start menu entry, as a trojan (Trojan:Win32/Bearfoos.A!ml).
    imp::stop_running(&exe)?;
    place::place(&exe)?;
    Ok(exe)
}

/// Installs the server that runs `exe`, from [`place()`]. Installing again restarts it.
pub(crate) fn install(listen: &str, exe: &Path) -> Result<Logged> {
    let file = imp::install(listen, exe, &serve_args())?.display().to_string();
    Ok(if with_computer() { Logged::Installed { file } } else { Logged::InstalledAtLogin { file } })
}

/// Root's server, and on Linux a headless user's, which `install` keeps running without a login.
#[cfg_attr(test, mutants::skip)] // Asks whether this process runs as root or without a login.
pub(crate) fn with_computer() -> bool {
    crate::sys::is_root() || imp::headless()
}

/// A server `uninstall` removed: its file, and whether it goes on running until logout.
pub(crate) struct Removed {
    file: String,
    running: bool,
}

pub(crate) fn uninstall() -> Result<Logged> {
    Ok(match imp::uninstall()? {
        Some(Removed { file, running: true }) => Logged::UninstalledRunning { file },
        Some(Removed { file, running: false }) => Logged::Uninstalled { file },
        None => Logged::NotInstalled,
    })
}

/// Whether the installed server starts at login; `None` if it is not installed.
#[cfg_attr(test, mutants::skip)] // Asks the service manager.
pub(crate) fn at_login() -> Option<bool> {
    imp::at_login()
}

/// Switches the installed server's start at login, and leaves the running one alone.
#[cfg_attr(test, mutants::skip)] // Changes the service manager's settings.
pub(crate) fn set_at_login(on: bool) -> Result<()> {
    imp::set_at_login(on)
}

/// Deletes the state directory: the runs and the login token. Never the profiles, nor
/// the repositories.
#[expect(clippy::disallowed_methods, reason = "writes the service files and runs the service manager")]
pub(crate) fn delete() -> Result<Option<Logged>> {
    let dir = crate::settings::state_dir()?;
    if !dir.exists() {
        return Ok(None);
    }
    std::fs::remove_dir_all(&dir)?;
    Ok(Some(Logged::Deleted { dir: dir.display().to_string() }))
}
