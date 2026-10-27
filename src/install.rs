//! Executables put on this computer: rustic's release when it is missing, rusticlone's own on update,
//! the folder they are in on `PATH`, and the server started with the computer.

#[cfg(feature = "self-update")]
mod attest;
#[cfg(feature = "desktop")]
pub(crate) mod autostart;
#[cfg(feature = "self-update")]
mod download;
pub(crate) mod on_path;
#[cfg(feature = "self-update")]
pub(crate) mod rustic;
#[cfg(feature = "self-update")]
pub(crate) mod update;
