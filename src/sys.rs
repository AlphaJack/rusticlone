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

/// Keeps the computer from sleeping when idle until dropped so that a run is not cut short. The system lets go
/// when this process ends however it does. Where it refuses, the computer may sleep as before.
#[cfg(all(feature = "server", target_vendor = "apple"))]
pub(crate) struct Awake(Option<u32>);

/// A power assertion of `IOKit`, as `caffeinate` takes.
#[cfg(all(feature = "server", target_vendor = "apple"))]
#[expect(unsafe_code, reason = "IOKit's power assertions are C functions")]
pub(crate) fn stay_awake() -> Awake {
    use std::ffi::{CString, c_char, c_void};
    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPMAssertionCreateWithName(kind: *const c_void, level: u32, name: *const c_void, id: *mut u32) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(allocator: *const c_void, text: *const c_char, encoding: u32) -> *const c_void;
        fn CFRelease(object: *const c_void);
    }
    const UTF8: u32 = 0x0800_0100;
    const ON: u32 = 255;
    let string = |text: &str| {
        let text = CString::new(text).unwrap_or_default();
        // SAFETY: a C string that lives through the call, and the default allocator.
        unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), UTF8) }
    };
    let (kind, name) = (string("PreventUserIdleSystemSleep"), string(&crate::i18n::t!("awake-why")));
    let mut id = 0;
    // SAFETY: two strings that the call copies, released after, and where it writes the assertion's id.
    let made = unsafe {
        let made = !kind.is_null() && !name.is_null() && IOPMAssertionCreateWithName(kind, ON, name, &raw mut id) == 0;
        for s in [kind, name].into_iter().filter(|s| !s.is_null()) {
            CFRelease(s);
        }
        made
    };
    Awake(made.then_some(id))
}

#[cfg(all(feature = "server", target_vendor = "apple"))]
#[expect(unsafe_code, reason = "IOKit's power assertions are C functions")]
impl Drop for Awake {
    fn drop(&mut self) {
        #[link(name = "IOKit", kind = "framework")]
        unsafe extern "C" {
            fn IOPMAssertionRelease(id: u32) -> i32;
        }
        if let Some(id) = self.0 {
            // SAFETY: an assertion this took and releases once.
            unsafe { IOPMAssertionRelease(id) };
        }
    }
}

/// A logind inhibitor, held by the file it hands over: closing it lets go.
#[cfg(all(feature = "server", target_os = "linux"))]
pub(crate) struct Awake(#[expect(dead_code, reason = "held until dropped")] Option<std::os::fd::OwnedFd>);

#[cfg(all(feature = "server", target_os = "linux"))]
pub(crate) fn stay_awake() -> Awake {
    let inhibit = async {
        let bus = zbus::Connection::system().await?;
        let (login, why) = ("org.freedesktop.login1", crate::i18n::t!("awake-why"));
        let args = ("idle:sleep", crate::APP_NAME, why.as_str(), "block");
        let reply = bus
            .call_method(
                Some(login),
                "/org/freedesktop/login1",
                Some("org.freedesktop.login1.Manager"),
                "Inhibit",
                &args,
            )
            .await?;
        Ok::<_, zbus::Error>(std::os::fd::OwnedFd::from(reply.body().deserialize::<zbus::zvariant::OwnedFd>()?))
    };
    // Called from a run's thread, outside the server's runtime.
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
    Awake(runtime.ok().and_then(|r| r.block_on(inhibit).ok()))
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

#[cfg(all(feature = "server", not(any(target_vendor = "apple", target_os = "linux", windows))))]
pub(crate) struct Awake(());

#[cfg(all(feature = "server", not(any(target_vendor = "apple", target_os = "linux", windows))))]
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

    /// What `held` finds within 5 seconds: logind takes a moment to list it.
    #[cfg(target_os = "linux")]
    #[expect(clippy::disallowed_methods, reason = "waits for another process, which wakes no clock")]
    fn held_soon(held: impl Fn() -> Option<Vec<String>>) -> Option<Vec<String>> {
        (0..50).find_map(|_| {
            std::thread::sleep(std::time::Duration::from_millis(100));
            held()
        })
    }

    /// The idle sleep assertions that this process holds, as `pmset` lists them.
    #[cfg(target_vendor = "apple")]
    fn assertions() -> Vec<String> {
        let all = output(&["/usr/bin/pmset", "-g", "assertions"]);
        let own = format!("pid {}(", std::process::id());
        all.lines()
            .filter(|l| l.contains(&own) && l.contains("PreventUserIdleSystemSleep"))
            .map(str::to_owned)
            .collect()
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn iokit_holds_an_idle_sleep_assertion_until_dropped() {
        let awake = stay_awake();
        assert!(awake.0.is_some(), "IOKit refused the assertion");
        let held = assertions();
        assert!(held.iter().any(|l| l.contains(&crate::i18n::t!("awake-why"))), "{held:?}");
        drop(awake);
        assert!(assertions().is_empty(), "the assertion outlived the run");
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
