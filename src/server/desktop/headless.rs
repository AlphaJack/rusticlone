//! Without a desktop, there is nothing to open a page or a folder on.

pub(super) fn open(_: &str, _: bool) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}
