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
pub(crate) fn on_battery() -> bool {
    power_source().as_deref() == Some("Battery Power")
}

/// The source that powers this computer as `IOKit` names it: `AC Power`, `Battery Power` or `UPS Power`.
#[cfg(all(feature = "server", target_vendor = "apple"))]
#[expect(unsafe_code, reason = "IOKit's power sources are C functions")]
fn power_source() -> Option<String> {
    use std::ffi::{CStr, c_char, c_void};
    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPSCopyPowerSourcesInfo() -> *const c_void;
        fn IOPSGetProvidingPowerSourceType(snapshot: *const c_void) -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringGetCString(string: *const c_void, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
        fn CFRelease(object: *const c_void);
    }
    const UTF8: u32 = 0x0800_0100;
    let mut buffer: [c_char; 64] = [0; 64];
    // SAFETY: a snapshot this owns and releases once the name it holds is copied into the buffer.
    unsafe {
        let snapshot = IOPSCopyPowerSourcesInfo();
        if snapshot.is_null() {
            return None;
        }
        let kind = IOPSGetProvidingPowerSourceType(snapshot);
        let size = isize::try_from(buffer.len()).unwrap_or_default();
        let read = !kind.is_null() && CFStringGetCString(kind, buffer.as_mut_ptr(), size, UTF8) != 0;
        CFRelease(snapshot);
        read.then(|| CStr::from_ptr(buffer.as_ptr()).to_string_lossy().into_owned())
    }
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

/// Whether `NetworkManager` says or guesses that the connection is metered: a phone's hotspot. False when
/// unknown, as without `NetworkManager`.
#[cfg(all(feature = "server", target_os = "linux"))]
pub(crate) fn metered() -> bool {
    network_manager_metered().is_some_and(metered_value)
}

/// `NetworkManager`'s `Metered` property, read on a thread of its own: callers may be on the server's runtime,
/// which cannot wait for another.
#[cfg(all(feature = "server", target_os = "linux"))]
fn network_manager_metered() -> Option<u32> {
    let read = async {
        let nm = "org.freedesktop.NetworkManager";
        let bus = zbus::Connection::system().await?;
        let proxy = zbus::Proxy::new(&bus, nm, "/org/freedesktop/NetworkManager", nm).await?;
        proxy.get_property::<u32>("Metered").await
    };
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
    std::thread::scope(|s| s.spawn(|| runtime.block_on(read).ok()).join().ok().flatten())
}

/// Whether a `Metered` value is metered: 1 is and 3 is guessed so. 2 and 4 are not metered. 0 is unknown.
#[cfg(all(feature = "server", any(test, target_os = "linux")))]
fn metered_value(value: u32) -> bool {
    matches!(value, 1 | 3)
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

/// Whether Windows says the connection costs: one set as metered or a cellular link.
#[cfg(all(feature = "server", windows))]
pub(crate) fn metered() -> bool {
    connection_cost().is_some_and(costs)
}

/// The machine's connection cost as the Network List Manager tells it: `NLM_CONNECTION_COST` flags.
#[cfg(all(feature = "server", windows))]
#[expect(unsafe_code, reason = "the Network List Manager is a COM object")]
fn connection_cost() -> Option<u32> {
    use std::ffi::c_void;
    use windows_sys::{
        Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize},
        core::GUID,
    };
    const NETWORK_LIST_MANAGER: GUID = GUID::from_u128(0xdcb0_0c01_570f_4a9b_8d69_199f_dba5_723b);
    const COST_MANAGER: GUID = GUID::from_u128(0xdcb0_0008_570f_4a9b_8d69_199f_dba5_723b);
    /// The start of `INetworkCostManager`'s table: `IUnknown`'s methods, then `GetCost`.
    #[repr(C)]
    struct Methods {
        query_interface: *const c_void,
        add_ref: *const c_void,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_cost: unsafe extern "system" fn(*mut c_void, *mut u32, *const c_void) -> i32,
    }
    // SAFETY: COM set up for this call on this thread, an object released once, and its cost written to `cost`.
    unsafe {
        let set_up = CoInitializeEx(std::ptr::null(), u32::try_from(COINIT_MULTITHREADED).unwrap_or_default()) >= 0;
        let mut manager: *mut c_void = std::ptr::null_mut();
        let mut cost = None;
        let made =
            CoCreateInstance(&NETWORK_LIST_MANAGER, std::ptr::null_mut(), CLSCTX_ALL, &COST_MANAGER, &raw mut manager);
        if made >= 0 && !manager.is_null() {
            let methods = *manager.cast::<*const Methods>();
            let mut value = 0;
            if ((*methods).get_cost)(manager, &raw mut value, std::ptr::null()) >= 0 {
                cost = Some(value);
            }
            ((*methods).release)(manager);
        }
        if set_up {
            CoUninitialize();
        }
        cost
    }
}

/// Whether `NLM_CONNECTION_COST` flags cost: fixed and variable do. Unrestricted is not metered.
#[cfg(all(feature = "server", any(test, windows)))]
fn costs(cost: u32) -> bool {
    /// `NLM_CONNECTION_COST_FIXED` and `NLM_CONNECTION_COST_VARIABLE`.
    const METERED: u32 = 0x2 | 0x4;
    cost & METERED != 0
}

#[cfg(all(feature = "server", not(any(target_os = "linux", target_vendor = "apple", windows))))]
pub(crate) fn metered() -> bool {
    false
}

#[cfg(all(test, feature = "server"))]
mod power_tests {
    use super::{costs, metered_value, supplies_on_battery};

    /// The CI's Macs and this one are plugged in or on battery: never unknown.
    #[cfg(target_vendor = "apple")]
    #[test]
    fn iokit_names_the_power_source() {
        let source = super::power_source();
        assert!(matches!(source.as_deref(), Some("AC Power" | "Battery Power" | "UPS Power")), "{source:?}");
    }

    /// Windows answers with the cost of the network it is on, or as unrestricted without one.
    #[cfg(windows)]
    #[test]
    fn the_network_list_manager_tells_the_cost() {
        let cost = super::connection_cost().expect("the Network List Manager did not answer");
        assert!(cost & 0x7 != 0, "no cost among the flags {cost:#x}");
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
        assert!(metered_value(1) && metered_value(3));
        assert!([0, 2, 4].into_iter().all(|v| !metered_value(v)));
    }

    /// Fixed and variable cost, alone or with roaming or a data limit near. Unrestricted does not.
    #[test]
    fn windows_says_whether_the_connection_costs() {
        assert!(costs(0x2) && costs(0x4) && costs(0x4 | 0x4_0000));
        assert!(!costs(0x1) && !costs(0));
    }
}
