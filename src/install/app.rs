//! The macOS app around an executable, which the package manager's executable was copied into: which one, and
//! what it held then. Elsewhere no executable is in an app, so there is none.

use std::path::{Path, PathBuf};

use crate::APP_NAME;

/// The file in the app's `Contents` that names the executable it copied.
pub(crate) fn source_file(contents: &Path) -> PathBuf {
    contents.join("Resources").join("source")
}

/// The package manager's executable that the app holding `exe` copied, and the SHA-256 it had then.
#[expect(clippy::disallowed_methods, reason = "reads what the app copied")]
pub(crate) fn source(exe: &Path) -> Option<(PathBuf, String)> {
    if !exe.ends_with(Path::new("Contents").join("MacOS").join(APP_NAME)) {
        return None;
    }
    let text = std::fs::read_to_string(source_file(exe.parent()?.parent()?)).ok()?;
    let mut lines = text.lines();
    Some((PathBuf::from(lines.next()?), lines.next().unwrap_or_default().to_owned()))
}

/// The package manager's executable that the app holding `exe` is a copy of: the package manager updates that one.
pub(crate) fn app_source(exe: &Path) -> Option<PathBuf> {
    source(exe).map(|(path, _)| path)
}
