//! What scheduled runs may wait for: this computer on battery, or on a connection that costs.
//!
//! Each system's calls are in a file of its own, used as `imp`.

mod changes;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(unix)]
mod unix;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

/// What scheduled runs may wait for, as this computer tells it. The engine asks through it so that tests can
/// answer instead.
pub(crate) trait Conditions: Send + Sync {
    /// Not plugged in. False when unknown, as on a desktop.
    fn on_battery(&self) -> bool;
    /// On a connection that costs, as [`watch_conditions`] last heard. False when unknown or before it watches.
    fn metered(&self) -> bool;
}

/// This computer, as its system tells it.
pub(crate) struct ThisComputer;

impl Conditions for ThisComputer {
    fn on_battery(&self) -> bool {
        imp::on_battery()
    }

    fn metered(&self) -> bool {
        imp::metered()
    }
}

/// Has the system call `on_change` whenever this computer goes on or off battery, or its connection's cost
/// changes. The first call starts watching. Later ones change nothing.
pub(crate) fn watch_conditions(on_change: impl Fn() + Send + Sync + 'static) {
    if changes::listen(Box::new(on_change)) {
        imp::watch_power();
        imp::watch_network();
    }
}
