//! Keeps the computer from sleeping when idle while a run lasts, so that it is not cut short. The system lets go
//! when this process ends however it does.
//!
//! Each system's calls are in a file of its own, used as `imp`.

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

pub(crate) use imp::stay_awake;
