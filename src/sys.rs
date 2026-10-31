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
/// on macOS and a systemd inhibitor on Linux. Each ends when its input closes: when dropped, and when this process
/// ends however it does. Where neither is found the computer may sleep as before.
#[cfg(all(feature = "server", unix))]
pub(crate) struct Awake(Option<std::process::Child>);

#[cfg(all(feature = "server", unix))]
#[expect(clippy::disallowed_methods, reason = "starts the system's sleep inhibitor, not rustic")]
pub(crate) fn stay_awake() -> Awake {
    use std::process::{Command, Stdio};
    let mut command = if cfg!(target_vendor = "apple") {
        // Until `cat` ends: `caffeinate` runs the command it is given and holds its assertion as long.
        let mut c = Command::new("/usr/bin/caffeinate");
        c.args(["-i", "cat"]);
        c
    } else {
        let mut c = Command::new("systemd-inhibit");
        let why = crate::i18n::t!("awake-why");
        c.args(["--what=idle:sleep", "--mode=block", &format!("--who={}", crate::APP_NAME), &format!("--why={why}")]);
        c.arg("cat");
        c
    };
    command.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
    Awake(command.spawn().ok())
}

#[cfg(all(feature = "server", unix))]
impl Drop for Awake {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            // `cat` ends at the end of its input, and the inhibitor with it.
            drop(child.stdin.take());
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

/// Each system says that something keeps it awake while held, and no longer once dropped.
#[cfg(all(test, feature = "server"))]
mod awake_tests {
    use super::stay_awake;

    /// What `command` prints.
    #[cfg(unix)]
    #[expect(clippy::disallowed_methods, reason = "asks the system what keeps it awake")]
    fn output(command: &[&str]) -> String {
        let out = std::process::Command::new(command[0]).args(&command[1..]).output();
        out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
    }

    /// What `held` finds within 5 seconds: the inhibitor takes a moment to start.
    #[cfg(unix)]
    #[expect(clippy::disallowed_methods, reason = "waits for another process, which wakes no clock")]
    fn held_soon(held: impl Fn() -> Option<Vec<String>>) -> Option<Vec<String>> {
        (0..50).find_map(|_| {
            std::thread::sleep(std::time::Duration::from_millis(100));
            held()
        })
    }

    /// The assertions that `caffeinate` holds on behalf of `pid`, as `pmset` lists them: it runs its command as
    /// that process and asserts from another.
    #[cfg(target_vendor = "apple")]
    fn assertions(pid: u32) -> Vec<String> {
        let all = output(&["/usr/bin/pmset", "-g", "assertions"]);
        let lines: Vec<&str> = all.lines().collect();
        let behalf = format!("on behalf of 'cat' (pid {pid})");
        lines.windows(2).filter(|w| w[1].contains(&behalf)).map(|w| w[0].to_owned()).collect()
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn caffeinate_holds_an_idle_sleep_assertion_until_dropped() {
        let awake = stay_awake();
        let pid = awake.0.as_ref().expect("caffeinate started").id();
        let held = held_soon(|| Some(assertions(pid)).filter(|a| !a.is_empty()));
        let held = held.expect("caffeinate holds no assertion");
        assert!(held.iter().any(|l| l.contains("PreventUserIdleSystemSleep")), "{held:?}");
        drop(awake);
        assert!(assertions(pid).is_empty(), "the assertion outlived the run");
    }

    /// The inhibitors that rusticlone holds, as logind lists them.
    #[cfg(target_os = "linux")]
    fn inhibitors() -> Vec<String> {
        let all = output(&["systemd-inhibit", "--list", "--no-pager", "--no-legend"]);
        all.lines().filter(|l| l.contains(crate::APP_NAME)).map(str::to_owned).collect()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn logind_lists_the_inhibitor_until_dropped() {
        let awake = stay_awake();
        let held = held_soon(|| Some(inhibitors()).filter(|i| !i.is_empty()));
        let held = held.expect("logind lists no inhibitor: systemd-inhibit is missing or was refused");
        assert!(held.iter().all(|l| l.contains("idle:sleep") && l.contains("block")), "{held:?}");
        drop(awake);
        assert!(inhibitors().is_empty(), "the inhibitor outlived the run");
    }

    #[cfg(windows)]
    #[test]
    #[expect(unsafe_code, reason = "SetThreadExecutionState takes flags only")]
    fn the_thread_requires_the_system_until_dropped() {
        use windows_sys::Win32::System::Power::{ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState};
        // Each call returns the state before it: each sets the same state again but the last.
        let state = |flags| unsafe { SetThreadExecutionState(flags) };
        let awake = stay_awake();
        assert_eq!(state(ES_CONTINUOUS | ES_SYSTEM_REQUIRED), ES_CONTINUOUS | ES_SYSTEM_REQUIRED);
        drop(awake);
        assert_eq!(state(ES_CONTINUOUS), ES_CONTINUOUS);
    }
}
