//! Facts about this computer: user, disks, mounts and paths.
//!
//! Each system's calls are in a file of its own, used as `os`. Every one of them defines the same functions, so
//! the code outside it needs no `cfg`.

use std::sync::LazyLock;

mod disk;
mod hash;
mod paths;

#[cfg(unix)]
mod unix;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as os;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as os;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as os;

pub(crate) use {disk::*, hash::*, os::*, paths::*};

/// The longest host name, with its NUL.
const HOSTNAME_BUFFER: usize = 256;

/// `None` if the system cannot tell.
pub(crate) fn hostname() -> Option<&'static str> {
    static NAME: LazyLock<Option<String>> = LazyLock::new(|| read_hostname().filter(|name| !name.is_empty()));
    NAME.as_deref()
}

/// A container this process runs in, as the file its tool puts there says.
#[cfg(any(feature = "desktop", feature = "self-update"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Container {
    Docker,
    Podman,
}

/// The container this process runs in: see [`Container`].
#[cfg(any(feature = "desktop", feature = "self-update"))]
#[cfg_attr(test, mutants::skip)] // Looks for files at the root of this computer, which tests cannot place.
#[expect(clippy::disallowed_methods, reason = "looks for the containers' files")]
pub(crate) fn container() -> Option<Container> {
    let [docker, podman] = container_markers()?;
    if docker.exists() { Some(Container::Docker) } else { podman.exists().then_some(Container::Podman) }
}

/// Keeps the computer from sleeping when idle until dropped so that a run is not cut short. It uses `caffeinate`
/// on macOS and a systemd inhibitor on Linux: both end with it. Where neither is found the
/// computer may sleep as before.
#[cfg(all(feature = "server", unix))]
pub(crate) struct Awake(Option<std::process::Child>);

#[cfg(all(feature = "server", unix))]
#[expect(clippy::disallowed_methods, reason = "starts the system's sleep inhibitor, not rustic")]
pub(crate) fn stay_awake() -> Awake {
    use std::process::{Command, Stdio};
    let mut command = if cfg!(target_vendor = "apple") {
        let mut c = Command::new("/usr/bin/caffeinate");
        c.arg("-i");
        c
    } else {
        let mut c = Command::new("systemd-inhibit");
        let why = crate::i18n::t!("awake-why");
        c.args(["--what=idle:sleep", "--mode=block", &format!("--who={}", crate::APP_NAME), &format!("--why={why}")]);
        c.args(["sleep", "infinity"]);
        c
    };
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    Awake(command.spawn().ok())
}

#[cfg(all(feature = "server", unix))]
impl Drop for Awake {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            _ = child.kill();
            _ = child.wait();
        }
    }
}

/// Windows keeps the computer awake while the thread that asked runs and until it asks no more.
#[cfg(all(feature = "server", windows))]
pub(crate) struct Awake(());

#[cfg(all(feature = "server", windows))]
#[expect(unsafe_code, reason = "SetThreadExecutionState takes flags only")]
pub(crate) fn stay_awake() -> Awake {
    use windows_sys::Win32::System::Power::{ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState};
    // SAFETY: flags only.
    unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
    Awake(())
}

#[cfg(all(feature = "server", windows))]
#[expect(unsafe_code, reason = "SetThreadExecutionState takes flags only")]
impl Drop for Awake {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Power::{ES_CONTINUOUS, SetThreadExecutionState};
        // SAFETY: flags only.
        unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
    }
}

#[cfg(all(feature = "server", not(any(unix, windows))))]
pub(crate) struct Awake(());

#[cfg(all(feature = "server", not(any(unix, windows))))]
pub(crate) fn stay_awake() -> Awake {
    Awake(())
}

#[cfg(all(test, feature = "server"))]
mod awake_tests {
    /// Asks the system to stay awake and then lets it sleep again.
    #[test]
    fn stays_awake_then_lets_go() {
        drop(super::stay_awake());
    }
}
