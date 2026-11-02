//! The code conventions in CONTRIBUTING that a test can check, read from the sources with `syn`, in one binary.

mod literals;
mod paths;
mod systems;

/// The crate's folder, which the sources are under.
#[expect(clippy::disallowed_methods, reason = "finds the sources")]
fn root() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR").map(std::path::PathBuf::from).unwrap_or_default()
}
