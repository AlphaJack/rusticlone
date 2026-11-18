//! The kernel's power supplies and `NetworkManager`'s cost of the connection.

use std::path::Path;

pub(super) use super::unix::metered;
use super::{changes::changed, unix::told_metered};

/// A battery discharging as the kernel reports power supplies.
pub(super) fn on_battery() -> bool {
    supplies_on_battery(&Path::new("/sys").join("class").join("power_supply"))
}

/// Whether the power supplies in `dir`, laid out as the kernel's, have a battery discharging and no mains plugged
/// in.
#[expect(clippy::disallowed_methods, reason = "reads the kernel's power supplies")]
fn supplies_on_battery(dir: &Path) -> bool {
    let read = |p: &Path, f: &str| std::fs::read_to_string(p.join(f)).unwrap_or_default().trim().to_owned();
    let supplies: Vec<_> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    let plugged = supplies.iter().any(|s| read(s, "type") == "Mains" && read(s, "online") == "1");
    !plugged && supplies.iter().any(|s| read(s, "type") == "Battery" && read(s, "status") == "Discharging")
}

/// The kernel's events for power supplies, which it sends as a supply is plugged, unplugged or changes state.
pub(super) fn watch_power() {
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
                changed();
            }
        }
    });
}

/// `NetworkManager`'s `Metered` property and its changes, on a thread of its own. Without `NetworkManager`,
/// as on servers and with `systemd-networkd`, the connection is never metered.
pub(super) fn watch_network() {
    use futures_util::StreamExt;
    let watch = async {
        let nm = "org.freedesktop.NetworkManager";
        let bus = zbus::Connection::system().await?;
        let proxy = zbus::Proxy::new(&bus, nm, "/org/freedesktop/NetworkManager", nm).await?;
        told_metered(metered_value(proxy.get_property("Metered").await?));
        let mut changes = proxy.receive_property_changed::<u32>("Metered").await;
        while let Some(change) = changes.next().await {
            told_metered(metered_value(change.get().await?));
        }
        Ok::<_, zbus::Error>(())
    };
    std::thread::spawn(|| {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
        _ = runtime.map(|r| r.block_on(watch));
    });
}

/// Whether a `Metered` value of `NetworkManager` is metered or guessed so. Others are not metered, or unknown.
fn metered_value(value: u32) -> bool {
    /// `NM_METERED_YES` and `NM_METERED_GUESS_YES`.
    const METERED: [u32; 2] = [1, 3];
    METERED.contains(&value)
}

#[cfg(test)]
mod tests {
    use super::{metered_value, supplies_on_battery};

    /// A laptop unplugged, then plugged in. A desktop has no battery. A missing folder is not on battery.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "writes power supplies as the kernel lays them out")]
    fn the_kernels_supplies_say_whether_on_battery() {
        let dir = crate::sys::test_dir("power-supplies");
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
