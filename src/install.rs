//! Executables put on this computer: rustic's release when it is missing, rusticlone's own on update,
//! the folder they are in on `PATH`, and the server started with the computer.

#[cfg(feature = "desktop")]
pub(crate) mod app;
#[cfg(feature = "self-update")]
mod attest;
#[cfg(feature = "desktop")]
pub(crate) mod autostart;
#[cfg(feature = "self-update")]
mod download;
#[cfg(any(feature = "desktop", feature = "self-update"))]
mod keeper;
pub(crate) mod path;
#[cfg(feature = "self-update")]
pub(crate) mod rustic;
#[cfg(feature = "self-update")]
pub(crate) mod update;

#[cfg(any(feature = "desktop", feature = "self-update"))]
pub(crate) use keeper::managed;
#[cfg(feature = "self-update")]
pub(crate) use keeper::{Keeper, keeper};
