//! What Linux's and macOS's service managers share: this process's `PATH`.

pub(super) fn path_var() -> String {
    std::env::var("PATH").unwrap_or_default()
}
