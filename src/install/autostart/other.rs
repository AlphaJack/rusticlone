use super::{Error, PathBuf, Result};

pub(super) fn install(_listen: &str, _exe: &std::path::Path, _serve: &[String]) -> Result<PathBuf> {
    Err(Error::Unsupported.into())
}

pub(super) fn uninstall() -> Result<Option<super::Logged>> {
    Err(Error::Unsupported.into())
}
