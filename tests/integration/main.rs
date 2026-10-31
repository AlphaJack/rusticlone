//! End-to-end tests driving the real `rustic` binary (needs `rustic` in PATH).
#![expect(clippy::unwrap_used, reason = "test assertions and helpers fail loudly")]
#![expect(clippy::disallowed_methods, reason = "end-to-end tests with sandboxes and real binaries")]

mod common;

mod backup;
#[cfg(feature = "server")]
mod client;
mod copy;
mod dry_run;
mod forget;
mod isolation;
#[cfg(feature = "server")]
mod kit;
mod maintenance;
mod quirks;
mod restore;
mod scheduling;
#[cfg(feature = "server")]
mod server;
mod status;
#[cfg(feature = "server")]
mod web;
