//! The power manager's power source and the IP helper's connectivity hint.

use std::ffi::c_void;

use super::changes::changed;

#[expect(unsafe_code, reason = "GetSystemPowerStatus fills the structure it is given")]
pub(super) fn on_battery() -> bool {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    // SAFETY: integers for which zeros are valid.
    let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    // SAFETY: a structure to fill. `ACLineStatus` is 0 offline and 1 online. 255 is unknown.
    unsafe { GetSystemPowerStatus(&raw mut status) != 0 && status.ACLineStatus == 0 }
}

/// The power source's setting, whose callback Windows calls when it changes.
#[expect(unsafe_code, reason = "registers a callback with the power manager")]
pub(super) fn watch_power() {
    use windows_sys::Win32::{
        System::{
            Power::{DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, PowerSettingRegisterNotification},
            SystemServices::GUID_ACDC_POWER_SOURCE,
        },
        UI::WindowsAndMessaging::DEVICE_NOTIFY_CALLBACK,
    };
    unsafe extern "system" fn power_changed(_: *const c_void, _: u32, _: *const c_void) -> u32 {
        changed();
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
#[expect(unsafe_code, reason = "registers a callback with the IP helper")]
pub(super) fn watch_network() {
    use windows_sys::Win32::{
        NetworkManagement::IpHelper::NotifyNetworkConnectivityHintChange,
        Networking::WinSock::NL_NETWORK_CONNECTIVITY_HINT,
    };
    unsafe extern "system" fn hint_changed(_: *const c_void, _: NL_NETWORK_CONNECTIVITY_HINT) {
        changed();
    }
    let mut handle = std::ptr::null_mut();
    // SAFETY: a callback without context, called at once and on each change, and where it writes the registration,
    // kept as long as the process runs.
    unsafe { NotifyNetworkConnectivityHintChange(Some(hint_changed), std::ptr::null(), true, &raw mut handle) };
}

/// Whether Windows says the connection costs: one set as metered or a cellular link.
#[expect(unsafe_code, reason = "GetNetworkConnectivityHint fills the structure it is given")]
pub(super) fn metered() -> bool {
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
    read && [NetworkConnectivityCostHintFixed, NetworkConnectivityCostHintVariable].contains(&hint.ConnectivityCost)
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
