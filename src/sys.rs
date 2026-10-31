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

/// What scheduled runs may wait for, as this computer tells it. The engine asks through it so that tests can
/// answer instead.
#[cfg(feature = "server")]
pub(crate) trait Conditions: Send + Sync {
    /// Not plugged in. False when unknown, as on a desktop.
    fn on_battery(&self) -> bool;
    /// On a connection that costs, as [`watch_conditions`] last heard. False when unknown or before it watches.
    fn metered(&self) -> bool;
}

/// This computer, as its system tells it.
#[cfg(feature = "server")]
pub(crate) struct ThisComputer;

#[cfg(feature = "server")]
impl Conditions for ThisComputer {
    fn on_battery(&self) -> bool {
        on_battery()
    }

    fn metered(&self) -> bool {
        metered()
    }
}

/// Called when this computer goes on or off battery, or its connection's cost changes.
#[cfg(feature = "server")]
static CHANGED: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

/// The connection's cost as the system last told it.
#[cfg(all(feature = "server", any(target_vendor = "apple", target_os = "linux")))]
static METERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "server")]
fn changed(metered: Option<bool>) {
    #[cfg(any(target_vendor = "apple", target_os = "linux"))]
    if let Some(metered) = metered {
        METERED.store(metered, std::sync::atomic::Ordering::Relaxed);
    }
    #[cfg(not(any(target_vendor = "apple", target_os = "linux")))]
    let _ = metered;
    if let Some(changed) = CHANGED.get() {
        changed();
    }
}

/// Has the system call `changed` whenever this computer goes on or off battery, or its connection's cost changes.
/// The first call starts watching. Later ones change nothing.
#[cfg(feature = "server")]
pub(crate) fn watch_conditions(on_change: impl Fn() + Send + Sync + 'static) {
    if CHANGED.set(Box::new(on_change)).is_ok() {
        watch_power();
        watch_network();
    }
}

#[cfg(all(feature = "server", target_vendor = "apple"))]
#[expect(unsafe_code, reason = "IOKit's estimate is a C function without arguments")]
fn on_battery() -> bool {
    /// What `IOKit` estimates on external power. Unknown or a number of seconds is on battery.
    const UNLIMITED: f64 = -2.0;
    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        safe fn IOPSGetTimeRemainingEstimate() -> f64;
    }
    IOPSGetTimeRemainingEstimate() > UNLIMITED
}

/// The power sources' notification, which `IOKit` posts when the source or the battery's state changes.
#[cfg(all(feature = "server", target_vendor = "apple"))]
#[expect(unsafe_code, reason = "registers for a system notification")]
fn watch_power() {
    use std::ffi::{c_char, c_void};
    unsafe extern "C" {
        fn notify_register_dispatch(
            name: *const c_char,
            token: *mut i32,
            queue: *mut c_void,
            handler: &block2::Block<dyn Fn(i32)>,
        ) -> u32;
        fn dispatch_get_global_queue(identifier: isize, flags: usize) -> *mut c_void;
    }
    let handler = block2::RcBlock::new(|_: i32| changed(None));
    let mut token = 0;
    // SAFETY: a name that lives through the call, the system's queue and a handler that the call copies.
    unsafe {
        notify_register_dispatch(
            c"com.apple.system.powersources.source".as_ptr(),
            &raw mut token,
            dispatch_get_global_queue(0, 0),
            &handler,
        );
    }
}

/// The Network framework's path monitor, which tells each change of path. A path is "expensive" on an iPhone's
/// hotspot or a cellular link, and "constrained" on a network the user put in Low Data Mode, which an Android
/// hotspot needs.
#[cfg(all(feature = "server", target_vendor = "apple"))]
#[expect(unsafe_code, reason = "starts the Network framework's path monitor")]
fn watch_network() {
    use std::ffi::c_void;
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
    let handler = block2::RcBlock::new(|path: *mut c_void| {
        // SAFETY: the path the monitor passes is valid during the call.
        changed(Some(unsafe { nw_path_is_expensive(path) || nw_path_is_constrained(path) }));
    });
    // SAFETY: a monitor that is never released so that it keeps running. The system's queue. The handler that the
    // monitor copies.
    unsafe {
        let monitor = nw_path_monitor_create();
        nw_path_monitor_set_queue(monitor, dispatch_get_global_queue(0, 0));
        nw_path_monitor_set_update_handler(monitor, &handler);
        nw_path_monitor_start(monitor);
    }
}

/// A battery discharging as the kernel reports power supplies.
#[cfg(all(feature = "server", target_os = "linux"))]
fn on_battery() -> bool {
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

/// The kernel's events for power supplies, which it sends as a supply is plugged, unplugged or changes state.
#[cfg(all(feature = "server", target_os = "linux"))]
fn watch_power() {
    use std::os::fd::AsRawFd;

    use nix::sys::socket::{
        AddressFamily, MsgFlags, NetlinkAddr, SockFlag, SockProtocol, SockType, bind, recv, socket,
    };
    /// The group of the kernel's device events.
    const KERNEL: u32 = 1;
    /// Larger than an event: the kernel caps them at a page.
    const EVENT: usize = 8192;
    std::thread::spawn(|| {
        let flags = SockFlag::SOCK_CLOEXEC;
        let Ok(events) = socket(AddressFamily::Netlink, SockType::Datagram, flags, SockProtocol::NetlinkKObjectUEvent)
        else {
            return;
        };
        if bind(events.as_raw_fd(), &NetlinkAddr::new(0, KERNEL)).is_err() {
            return;
        }
        let mut buffer = vec![0; EVENT];
        while let Ok(read) = recv(events.as_raw_fd(), &mut buffer, MsgFlags::empty()) {
            let event = buffer.get(..read).unwrap_or_default();
            if event.split(|b| *b == 0).any(|field| field == b"SUBSYSTEM=power_supply") {
                changed(None);
            }
        }
    });
}

/// `NetworkManager`'s `Metered` property and its changes, on a thread of its own. Without `NetworkManager`,
/// as on servers and with `systemd-networkd`, the connection is never metered.
#[cfg(all(feature = "server", target_os = "linux"))]
fn watch_network() {
    use futures_util::StreamExt;
    let watch = async {
        let nm = "org.freedesktop.NetworkManager";
        let bus = zbus::Connection::system().await?;
        let proxy = zbus::Proxy::new(&bus, nm, "/org/freedesktop/NetworkManager", nm).await?;
        changed(Some(metered_value(proxy.get_property("Metered").await?)));
        let mut changes = proxy.receive_property_changed::<u32>("Metered").await;
        while let Some(change) = changes.next().await {
            changed(Some(metered_value(change.get().await?)));
        }
        Ok::<_, zbus::Error>(())
    };
    std::thread::spawn(|| {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
        _ = runtime.map(|r| r.block_on(watch));
    });
}

/// Whether a `Metered` value of `NetworkManager` is metered or guessed so. Others are not metered, or unknown.
#[cfg(all(feature = "server", any(test, target_os = "linux")))]
fn metered_value(value: u32) -> bool {
    /// `NM_METERED_YES` and `NM_METERED_GUESS_YES`.
    const METERED: [u32; 2] = [1, 3];
    METERED.contains(&value)
}

#[cfg(all(feature = "server", any(target_vendor = "apple", target_os = "linux")))]
fn metered() -> bool {
    METERED.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(all(feature = "server", windows))]
#[expect(unsafe_code, reason = "GetSystemPowerStatus fills the structure it is given")]
fn on_battery() -> bool {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    // SAFETY: integers for which zeros are valid.
    let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    // SAFETY: a structure to fill. `ACLineStatus` is 0 offline and 1 online. 255 is unknown.
    unsafe { GetSystemPowerStatus(&raw mut status) != 0 && status.ACLineStatus == 0 }
}

/// The power source's setting, whose callback Windows calls when it changes.
#[cfg(all(feature = "server", windows))]
#[expect(unsafe_code, reason = "registers a callback with the power manager")]
fn watch_power() {
    use std::ffi::c_void;
    use windows_sys::Win32::{
        System::{
            Power::{DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, PowerSettingRegisterNotification},
            SystemServices::GUID_ACDC_POWER_SOURCE,
        },
        UI::WindowsAndMessaging::DEVICE_NOTIFY_CALLBACK,
    };
    unsafe extern "system" fn power_changed(_: *const c_void, _: u32, _: *const c_void) -> u32 {
        changed(None);
        0
    }
    // Kept for as long as the process runs, which the registration is.
    let recipient = Box::leak(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
        Callback: Some(power_changed),
        Context: std::ptr::null_mut(),
    }));
    let mut registration = std::ptr::null_mut();
    // SAFETY: a recipient that lives as long as the process, and where it writes the registration.
    unsafe {
        PowerSettingRegisterNotification(
            &GUID_ACDC_POWER_SOURCE,
            DEVICE_NOTIFY_CALLBACK,
            std::ptr::from_mut(recipient).cast(),
            &raw mut registration,
        );
    }
}

/// The connectivity hint, whose callback Windows calls when the connection or its cost changes.
#[cfg(all(feature = "server", windows))]
#[expect(unsafe_code, reason = "registers a callback with the IP helper")]
fn watch_network() {
    use std::ffi::c_void;
    use windows_sys::Win32::{
        NetworkManagement::IpHelper::NotifyNetworkConnectivityHintChange,
        Networking::WinSock::NL_NETWORK_CONNECTIVITY_HINT,
    };
    unsafe extern "system" fn hint_changed(_: *const c_void, _: NL_NETWORK_CONNECTIVITY_HINT) {
        changed(None);
    }
    let mut handle = std::ptr::null_mut();
    // SAFETY: a callback without context, called at once and on each change, and where it writes the registration,
    // kept as long as the process runs.
    unsafe { NotifyNetworkConnectivityHintChange(Some(hint_changed), std::ptr::null(), true, &raw mut handle) };
}

/// Whether Windows says the connection costs: one set as metered or a cellular link.
#[cfg(all(feature = "server", windows))]
#[expect(unsafe_code, reason = "GetNetworkConnectivityHint fills the structure it is given")]
fn metered() -> bool {
    use windows_sys::Win32::{
        NetworkManagement::IpHelper::GetNetworkConnectivityHint,
        Networking::WinSock::{
            NL_NETWORK_CONNECTIVITY_HINT, NetworkConnectivityCostHintFixed, NetworkConnectivityCostHintVariable,
        },
    };
    // SAFETY: integers and booleans for which zeros are valid.
    let mut hint: NL_NETWORK_CONNECTIVITY_HINT = unsafe { std::mem::zeroed() };
    // SAFETY: a structure to fill.
    let read = unsafe { GetNetworkConnectivityHint(&raw mut hint) } == 0;
    read && matches!(hint.ConnectivityCost, NetworkConnectivityCostHintFixed | NetworkConnectivityCostHintVariable)
}

#[cfg(all(feature = "server", not(any(target_vendor = "apple", target_os = "linux", windows))))]
fn on_battery() -> bool {
    false
}

#[cfg(all(feature = "server", not(any(target_vendor = "apple", target_os = "linux", windows))))]
fn metered() -> bool {
    false
}

#[cfg(all(feature = "server", not(any(target_vendor = "apple", target_os = "linux", windows))))]
fn watch_power() {}

#[cfg(all(feature = "server", not(any(target_vendor = "apple", target_os = "linux", windows))))]
fn watch_network() {}

#[cfg(all(test, feature = "server"))]
mod power_tests {
    use super::{metered_value, supplies_on_battery};

    /// The network monitor answers once it starts: with the connection's cost, and a change to wake runs.
    #[cfg(any(target_vendor = "apple", windows))]
    #[test]
    fn the_system_tells_when_the_network_is_known() {
        let (told, heard) = std::sync::mpsc::channel();
        super::watch_conditions(move || _ = told.send(()));
        heard.recv_timeout(std::time::Duration::from_secs(5)).expect("the system told nothing");
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
}
