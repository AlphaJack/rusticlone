//! Shared by Linux and macOS: the `PATH` the server runs with.

pub(super) fn path_var() -> String {
    std::env::var("PATH").unwrap_or_default()
}
