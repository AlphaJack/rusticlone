//! `IOKit`'s power sources and the Network framework's path monitor.

use std::ffi::c_void;

pub(super) use super::unix::metered;
use super::{changes::changed, unix::told_metered};

#[expect(unsafe_code, reason = "IOKit's estimate is a C function without arguments")]
pub(super) fn on_battery() -> bool {
    /// What `IOKit` estimates on external power. Unknown or a number of seconds is on battery.
    const UNLIMITED: f64 = -2.0;
    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        safe fn IOPSGetTimeRemainingEstimate() -> f64;
    }
    IOPSGetTimeRemainingEstimate() > UNLIMITED
}

/// The system's queue of default priority, on which the watches' handlers run.
#[expect(unsafe_code, reason = "dispatch's queues are C functions")]
fn queue() -> *mut c_void {
    unsafe extern "C" {
        safe fn dispatch_get_global_queue(identifier: isize, flags: usize) -> *mut c_void;
    }
    dispatch_get_global_queue(0, 0)
}

/// The power sources' notification, which `IOKit` posts when the source or the battery's state changes.
#[expect(unsafe_code, reason = "registers for a system notification")]
pub(super) fn watch_power() {
    use std::ffi::c_char;
    unsafe extern "C" {
        fn notify_register_dispatch(
            name: *const c_char,
            token: *mut i32,
            queue: *mut c_void,
            handler: &block2::Block<dyn Fn(i32)>,
        ) -> u32;
    }
    let handler = block2::RcBlock::new(|_: i32| changed());
    let mut token = 0;
    // SAFETY: a name that lives through the call, the system's queue and a handler that the call copies.
    unsafe {
        notify_register_dispatch(c"com.apple.system.powersources.source".as_ptr(), &raw mut token, queue(), &handler);
    }
}

/// The Network framework's path monitor, which tells each change of path. A path is "expensive" on an iPhone's
/// hotspot or a cellular link, and "constrained" on a network the user put in Low Data Mode, which an Android
/// hotspot needs.
#[expect(unsafe_code, reason = "starts the Network framework's path monitor")]
pub(super) fn watch_network() {
    #[link(name = "Network", kind = "framework")]
    unsafe extern "C" {
        fn nw_path_monitor_create() -> *mut c_void;
        fn nw_path_monitor_set_queue(monitor: *mut c_void, queue: *mut c_void);
        fn nw_path_monitor_set_update_handler(monitor: *mut c_void, handler: &block2::Block<dyn Fn(*mut c_void)>);
        fn nw_path_monitor_start(monitor: *mut c_void);
        fn nw_path_is_expensive(path: *mut c_void) -> bool;
        fn nw_path_is_constrained(path: *mut c_void) -> bool;
    }
    let handler = block2::RcBlock::new(|path: *mut c_void| {
        // SAFETY: the path the monitor passes is valid during the call.
        told_metered(unsafe { nw_path_is_expensive(path) || nw_path_is_constrained(path) });
    });
    // SAFETY: a monitor that is never released so that it keeps running. The system's queue. The handler that the
    // monitor copies.
    unsafe {
        let monitor = nw_path_monitor_create();
        nw_path_monitor_set_queue(monitor, queue());
        nw_path_monitor_set_update_handler(monitor, &handler);
        nw_path_monitor_start(monitor);
    }
}

#[cfg(test)]
mod tests {
    /// The network monitor answers once it starts: with the connection's cost, and a change to wake runs.
    #[test]
    fn the_system_tells_when_the_network_is_known() {
        let (told, heard) = std::sync::mpsc::channel();
        crate::sys::watch_conditions(move || _ = told.send(()));
        heard.recv_timeout(std::time::Duration::from_secs(5)).expect("the system told nothing");
    }
}
