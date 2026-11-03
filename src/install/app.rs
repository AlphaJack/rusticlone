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

#[cfg(test)]
mod tests {
    #![expect(clippy::disallowed_methods, reason = "lays out an app")]

    use super::{APP_NAME, app_source, source};

    /// The app names the executable it copied, and the SHA-256 it had then, in its `Contents`. An executable outside
    /// an app has no source.
    #[test]
    fn the_app_names_what_it_copied() {
        let app = crate::sys::test_dir("app").join(format!("{APP_NAME}.app")).join("Contents");
        let exe = app.join("MacOS").join(APP_NAME);
        std::fs::create_dir_all(app.join("Resources")).unwrap();
        let copied = std::path::Path::new("/opt").join("homebrew").join("bin").join("rusticlone");
        std::fs::write(app.join("Resources").join("source"), format!("{}\nabc123\n", copied.display())).unwrap();
        assert_eq!(source(&exe), Some((copied.clone(), "abc123".into())));
        assert_eq!(app_source(&exe), Some(copied));
        assert_eq!(source(&app.join("MacOS").join("other")), None, "not the app's executable");
    }
}
