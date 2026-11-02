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

#[cfg(target_vendor = "apple")]
mod macos;
#[cfg(target_vendor = "apple")]
use macos as os;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as os;

#[cfg(all(unix, not(any(target_os = "linux", target_vendor = "apple"))))]
mod other;
#[cfg(all(unix, not(any(target_os = "linux", target_vendor = "apple"))))]
use other as os;

pub(crate) use {disk::*, hash::*, os::*, paths::*};

/// The longest host name, with its NUL.
const HOSTNAME_BUFFER: usize = 256;

/// Empty if the system cannot tell.
pub(crate) fn hostname() -> &'static str {
    static NAME: LazyLock<String> = LazyLock::new(|| read_hostname().unwrap_or_default());
    &NAME
}
