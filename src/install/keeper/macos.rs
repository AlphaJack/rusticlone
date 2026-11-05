//! macOS's keepers: the app's copy of a package manager's executable.

use std::path::Path;

use super::managed;

/// The app copies Homebrew's executable, and names it in its resources.
#[test]
#[expect(clippy::disallowed_methods, reason = "lays out the app")]
fn the_macos_apps_copy_of_homebrews_executable_is_managed() {
    let app = std::env::temp_dir().join(format!("{}-app-{}.app", crate::NAME, std::process::id()));
    let exe = app.join("Contents").join("MacOS").join(crate::APP_NAME);
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::create_dir_all(app.join("Contents").join("Resources")).unwrap();
    std::fs::write(&exe, b"").unwrap();
    let mine = managed(&exe);
    std::fs::write(app.join("Contents").join("Resources").join("source"), "/opt/homebrew/bin/rusticlone\nabc123\n")
        .unwrap();
    let source = crate::install::app::app_source(&exe);
    let found = managed(&exe);
    std::fs::remove_dir_all(&app).unwrap();
    assert!(!mine && found);
    assert_eq!(source, Some(Path::new("/opt").join("homebrew").join("bin").join("rusticlone")));
}
