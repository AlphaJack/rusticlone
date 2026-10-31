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

/// Whether this computer runs on battery: not plugged in. False when unknown as on a desktop.
#[cfg(all(feature = "server", target_vendor = "apple"))]
#[expect(clippy::disallowed_methods, reason = "asks the power manager, not rustic")]
pub(crate) fn on_battery() -> bool {
    let out = std::process::Command::new("/usr/bin/pmset").args(["-g", "ps"]).output();
    out.is_ok_and(|o| pmset_on_battery(&String::from_utf8_lossy(&o.stdout)))
}

/// Whether `pmset -g ps` says the power comes from the battery.
#[cfg(all(feature = "server", any(test, target_vendor = "apple")))]
fn pmset_on_battery(out: &str) -> bool {
    out.contains("'Battery Power'")
}

/// A battery discharging as the kernel reports power supplies.
#[cfg(all(feature = "server", target_os = "linux"))]
pub(crate) fn on_battery() -> bool {
    supplies_on_battery(std::path::Path::new("/sys/class/power_supply"))
}

/// Whether the power supplies in `dir`, laid out as the kernel's, have a battery discharging and no mains plugged
/// in.
#[cfg(all(feature = "server", any(test, target_os = "linux")))]
#[expect(clippy::disallowed_methods, reason = "reads the kernel's power supplies")]
fn supplies_on_battery(dir: &std::path::Path) -> bool {
    let read = |p: &std::path::Path, f: &str| std::fs::read_to_string(p.join(f)).unwrap_or_default().trim().to_owned();
    let supplies: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    let plugged = supplies.iter().any(|s| read(s, "type") == "Mains" && read(s, "online") == "1");
    !plugged && supplies.iter().any(|s| read(s, "type") == "Battery" && read(s, "status") == "Discharging")
}

#[cfg(all(feature = "server", windows))]
#[expect(unsafe_code, reason = "GetSystemPowerStatus fills the structure it is given")]
pub(crate) fn on_battery() -> bool {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    // SAFETY: integers for which zeros are valid.
    let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    // SAFETY: a structure to fill. `ACLineStatus` is 0 offline and 1 online. 255 is unknown.
    unsafe { GetSystemPowerStatus(&raw mut status) != 0 && status.ACLineStatus == 0 }
}

#[cfg(all(feature = "server", not(any(target_vendor = "apple", target_os = "linux", windows))))]
pub(crate) fn on_battery() -> bool {
    false
}

/// Whether NetworkManager says or guesses that the connection is metered: a phone's hotspot. False when
/// unknown.
#[cfg(all(feature = "server", target_os = "linux"))]
#[expect(clippy::disallowed_methods, reason = "asks NetworkManager, not rustic")]
pub(crate) fn metered() -> bool {
    let nm = "org.freedesktop.NetworkManager";
    let out = std::process::Command::new("busctl")
        .args(["get-property", nm, "/org/freedesktop/NetworkManager", nm, "Metered"])
        .output();
    out.is_ok_and(|o| busctl_metered(&String::from_utf8_lossy(&o.stdout)))
}

/// Whether `NetworkManager`'s `Metered` property, as `busctl` prints it, is metered: `u 1` is metered and `u 3`
/// guessed so. 2 and 4 are not metered. 0 is unknown.
#[cfg(all(feature = "server", any(test, target_os = "linux")))]
fn busctl_metered(out: &str) -> bool {
    matches!(out.trim(), "u 1" | "u 3")
}

/// Whether macOS says the connection costs. It is "expensive" on an iPhone's hotspot or a cellular link. It is
/// "constrained" on a network the user put in Low Data Mode. An Android hotspot needs that. The Network framework
/// tells a monitor that keeps running. The first call starts it and waits briefly for its first answer.
#[cfg(all(feature = "server", target_vendor = "apple"))]
#[expect(unsafe_code, reason = "starts the Network framework's path monitor")]
pub(crate) fn metered() -> bool {
    use std::{ffi::c_void, sync::OnceLock, time::Duration};
    /// How long the first call waits for the monitor's first answer.
    const FIRST_ANSWER: Duration = Duration::from_secs(1);
    #[link(name = "Network", kind = "framework")]
    unsafe extern "C" {
        fn nw_path_monitor_create() -> *mut c_void;
        fn nw_path_monitor_set_queue(monitor: *mut c_void, queue: *mut c_void);
        fn nw_path_monitor_set_update_handler(monitor: *mut c_void, handler: &block2::Block<dyn Fn(*mut c_void)>);
        fn nw_path_monitor_start(monitor: *mut c_void);
        fn nw_path_is_expensive(path: *mut c_void) -> bool;
        fn nw_path_is_constrained(path: *mut c_void) -> bool;
        fn dispatch_get_global_queue(identifier: isize, flags: usize) -> *mut c_void;
    }
    /// The last answer. `None` until the first one.
    static METERED: parking_lot::Mutex<Option<bool>> = parking_lot::Mutex::new(None);
    static ANSWERED: parking_lot::Condvar = parking_lot::Condvar::new();
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        let handler = block2::RcBlock::new(|path: *mut c_void| {
            // SAFETY: the path the monitor passes is valid during the call.
            let costs = unsafe { nw_path_is_expensive(path) || nw_path_is_constrained(path) };
            *METERED.lock() = Some(costs);
            ANSWERED.notify_all();
        });
        // SAFETY: a monitor that is never released so that it keeps running. The system's queue. The
        // handler that the monitor copies.
        unsafe {
            let monitor = nw_path_monitor_create();
            nw_path_monitor_set_queue(monitor, dispatch_get_global_queue(0, 0));
            nw_path_monitor_set_update_handler(monitor, &handler);
            nw_path_monitor_start(monitor);
        }
    });
    let mut metered = METERED.lock();
    if metered.is_none() {
        ANSWERED.wait_for(&mut metered, FIRST_ANSWER);
    }
    metered.unwrap_or_default()
}

/// Whether Windows says the connection costs: one set as metered or a cellular link. The WinRT network
/// information reports it. Windows PowerShell reads it without other dependencies.
#[cfg(all(feature = "server", windows))]
#[expect(clippy::disallowed_methods, reason = "asks Windows for the connection's cost, not rustic")]
pub(crate) fn metered() -> bool {
    use std::os::windows::process::CommandExt;
    /// Started without a console window that would flash on the desktop.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const COST: &str = "$p = [Windows.Networking.Connectivity.NetworkInformation, Windows.Networking.Connectivity, \
        ContentType=WindowsRuntime]::GetInternetConnectionProfile(); if ($p) { $p.GetConnectionCost().NetworkCostType }";
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", COST])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    out.is_ok_and(|o| cost_metered(&String::from_utf8_lossy(&o.stdout)))
}

/// Whether a `NetworkCostType` costs: `Fixed` and `Variable` do. `Unrestricted` is not metered. `Unknown` means
/// there is no connection.
#[cfg(all(feature = "server", any(test, windows)))]
fn cost_metered(out: &str) -> bool {
    matches!(out.trim(), "Fixed" | "Variable")
}

#[cfg(all(feature = "server", not(any(target_os = "linux", target_vendor = "apple", windows))))]
pub(crate) fn metered() -> bool {
    false
}

#[cfg(all(test, feature = "server"))]
mod power_tests {
    use super::{busctl_metered, cost_metered, pmset_on_battery, supplies_on_battery};

    #[test]
    fn pmset_says_where_power_comes_from() {
        let battery = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=4653155)\t87%; discharging;";
        let plugged = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=4653155)\t87%; charging;";
        assert!(pmset_on_battery(battery) && !pmset_on_battery(plugged) && !pmset_on_battery(""));
    }

    /// A laptop unplugged, then plugged in. A desktop has no battery. A missing folder is not on battery.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "writes power supplies as the kernel lays them out")]
    fn the_kernels_supplies_say_whether_on_battery() {
        let dir = super::test_dir("power-supplies");
        let supply = |name: &str, files: &[(&str, &str)]| {
            let path = dir.join(name);
            std::fs::create_dir_all(&path).unwrap();
            for (file, value) in files {
                std::fs::write(path.join(file), format!("{value}\n")).unwrap();
            }
        };
        supply("BAT0", &[("type", "Battery"), ("status", "Discharging")]);
        supply("AC", &[("type", "Mains"), ("online", "0")]);
        assert!(supplies_on_battery(&dir));
        supply("AC", &[("type", "Mains"), ("online", "1")]);
        assert!(!supplies_on_battery(&dir));
        std::fs::remove_dir_all(dir.join("BAT0")).unwrap();
        supply("AC", &[("type", "Mains"), ("online", "0")]);
        assert!(!supplies_on_battery(&dir));
        assert!(!supplies_on_battery(&dir.join("none")));
    }

    #[test]
    fn networkmanager_says_whether_metered() {
        assert!(busctl_metered("u 1\n") && busctl_metered("u 3"));
        assert!(["u 0", "u 2", "u 4", ""].iter().all(|o| !busctl_metered(o)));
    }

    #[test]
    fn windows_says_whether_the_connection_costs() {
        assert!(cost_metered("Fixed\r\n") && cost_metered("Variable"));
        assert!(["Unrestricted", "Unknown", ""].iter().all(|o| !cost_metered(o)));
    }
}
