use std::{
    fs, thread,
    time::{Duration, Instant},
};

use rusticlone::{Event, Status};
use serde_json::json;

use crate::{
    client::Server,
    common::{
        KEEP, NAME, Sandbox, cmd, docs_with_remote, global_env, json_has, noise, slow_profile, stdin_profile, var,
        wait_for,
    },
};

mod auth;
#[cfg(target_os = "linux")]
mod desktop;
#[cfg(unix)]
mod install;
mod queue;
mod runs;

/// The card with this `id` on a page.
pub(crate) fn card(page: &str, id: &str) -> String {
    shown_card(page, id).unwrap_or_else(|| panic!("no {id} in {page}"))
}

/// [`card`], if the page shows it yet.
fn shown_card(page: &str, id: &str) -> Option<String> {
    let start = page.find(&format!(r#"id="{id}""#))?;
    let end = page.get(start..)?.find("</section>").map_or(page.len(), |e| start + e);
    Some(page.get(start..end)?.to_owned())
}

/// A server, and a command line backup of collection Slow that it sees running.
fn backing_up_elsewhere() -> (Sandbox, Server, std::process::Child) {
    let sb = Sandbox::new();
    slow_profile(&sb, "Slow", 3, "");
    let server = sb.server(&[]);
    let cli = sb.spawn(&["backup", "--collection", "Slow"], &[]);
    wait_for(20, "the run of the other process", || server.collection("Slow")["running"][0]["elsewhere"] == true);
    (sb, server, cli)
}

/// A folder on another disk than `here`, if there is one: `/tmp` or `/var/tmp`, or the root of another fixed
/// drive on Windows.
fn other_disk(here: &std::path::Path) -> Option<std::path::PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let device = |p: &std::path::Path| fs::metadata(p).unwrap().dev();
        ["/tmp", "/var/tmp"].map(std::path::PathBuf::from).into_iter().find(|d| device(d) != device(here))
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
        // `GetDriveTypeW`'s answer for a hard disk or a solid-state drive.
        const DRIVE_FIXED: u32 = 3;
        let fixed = |root: &str| {
            let wide: Vec<u16> = root.encode_utf16().chain([0]).collect();
            // SAFETY: a NUL-terminated string.
            #[expect(unsafe_code, reason = "asks a drive's type")]
            let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
            kind == DRIVE_FIXED
        };
        let here = here.components().next();
        let roots = (b'C'..=b'Z').map(|letter| format!("{}:\\", char::from(letter)));
        roots.filter(|root| fixed(root)).map(std::path::PathBuf::from).find(|root| root.components().next() != here)
    }
}

/// The service manager that `install` reaches, which only records what it is asked in `log`, so that the
/// tests touch no real one. On macOS, a `launchctl` first on `PATH`. On Linux, a D-Bus of its own with
/// systemd and logind. None for root, whose install writes to the system's folders, which no sandbox holds.
#[cfg(unix)]
#[expect(clippy::disallowed_methods, reason = "asks the system who runs the tests")]
fn fake_service_manager(sb: &Sandbox, log: &std::path::Path) -> Option<Fake> {
    if std::process::Command::new("id").arg("-u").output().is_ok_and(|o| o.stdout == b"0\n") {
        eprintln!("skipped: as root, install writes to the system's folders");
        return None;
    }
    let path = std::env::var("PATH").unwrap();
    #[cfg(target_os = "linux")]
    return Some(Fake { path, bus: crate::bus::FakeBus::start(sb, log, None) });
    #[cfg(not(target_os = "linux"))]
    {
        use std::os::unix::fs::PermissionsExt;
        let bin = sb.path("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("launchctl"), format!("#!/bin/sh\necho \"$*\" >> {}\n", log.display())).unwrap();
        fs::set_permissions(bin.join("launchctl"), fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&path)));
        Some(Fake { path: path.unwrap().into_string().unwrap() })
    }
}

/// What `fake_service_manager` sets up, kept for as long as the test runs.
#[cfg(unix)]
struct Fake {
    path: String,
    #[cfg(target_os = "linux")]
    bus: crate::bus::FakeBus,
}

#[cfg(unix)]
impl Fake {
    /// The variables that send `install` to the fake.
    fn env(&self) -> Vec<(&str, &str)> {
        #[cfg(target_os = "linux")]
        let bus = self.bus.env();
        #[cfg(not(target_os = "linux"))]
        let bus: [(&str, &str); 0] = [];
        std::iter::once(("PATH", self.path.as_str())).chain(bus).collect()
    }
}

/// What `install` left in `dir`: files named after rusticlone.
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
fn leftovers(dir: &std::path::Path) -> Vec<String> {
    let entries = fs::read_dir(dir).into_iter().flatten().flatten();
    entries.map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.contains(NAME)).collect()
}
