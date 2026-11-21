//! Without a desktop, there is nothing to open a page or a folder on.

pub(super) fn open(_: &str, _: bool) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

#[expect(clippy::unused_async, reason = "as Linux's, which asks over D-Bus")]
pub(super) async fn reveal(_: &std::path::Path) -> bool {
    false
}
