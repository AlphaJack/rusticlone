//! Shared by Linux and macOS: the `PATH` the server runs with.

#[expect(clippy::disallowed_methods, reason = "writes the service files and runs the service manager")]
pub(super) fn path_var() -> String {
    std::env::var("PATH").unwrap_or_default()
}
