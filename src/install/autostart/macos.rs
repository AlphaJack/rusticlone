use super::{Path, PathBuf, Removed, Result, unix::path_var};
use crate::{APP_NAME, NAME, sys};

#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "autostart-error")]
pub(crate) enum Error {
    CannotRun { program: String, source: std::io::Error },
    Failed { command: String, error: String },
}

/// Runs a command of macOS's that has no API, as launchd's and codesign's, failing with its output.
fn run(program: &str, args: &[&str]) -> Result<()> {
    let out = (std::process::Command::new(program).args(args).output())
        .map_err(|source| Error::CannotRun { program: program.to_owned(), source })?;
    if !out.status.success() {
        let (command, output) = (format!("{program} {}", args.join(" ")), String::from_utf8_lossy(&out.stderr));
        return Err(Error::Failed { command, error: output.trim().to_owned() }.into());
    }
    Ok(())
}

/// What macOS keys the user's approval and the app's permissions on: it never changes.
const BUNDLE_ID: &str = "io.github.alphajack.rusticlone";

/// A binary property list holding one array of short ASCII strings, as macOS keeps a file's Spotlight
/// keywords: the header, the array of references to the strings, the strings, the offset of each, and the
/// trailer that says how wide offsets and references are, how many objects there are and where the offsets
/// are. `None` if a count, length or offset needs more than a byte.
pub(super) fn bplist(strings: &[&str]) -> Option<Vec<u8>> {
    /// A marker's low nibble holds a count or a length below this, which says that a longer one follows.
    const SHORT: u8 = 15;
    /// The markers of an array and of an ASCII string.
    const ARRAY: u8 = 0xA0;
    const ASCII: u8 = 0x50;
    // One byte holds a count or a length below SHORT, an object reference, and an offset below 256.
    let short = |n: usize| u8::try_from(n).ok().filter(|n| *n < SHORT);
    let count = short(strings.len())?;
    let mut plist = b"bplist00".to_vec();
    let mut offsets = vec![u8::try_from(plist.len()).ok()?];
    plist.push(ARRAY | count);
    plist.extend(1..=count);
    for string in strings {
        offsets.push(u8::try_from(plist.len()).ok()?);
        plist.push(ASCII | short(string.len()).filter(|_| string.is_ascii())?);
        plist.extend(string.as_bytes());
    }
    let table = u64::try_from(plist.len()).ok()?;
    plist.extend(offsets);
    plist.extend([0, 0, 0, 0, 0, 0, 1, 1]);
    for n in [u64::from(count) + 1, 0, table] {
        plist.extend(n.to_be_bytes());
    }
    Some(plist)
}

fn plist() -> Result<PathBuf> {
    Ok(if sys::is_root() {
        PathBuf::from("/Library").join("LaunchDaemons")
    } else {
        etcetera::home_dir()?.join("Library").join("LaunchAgents")
    }
    .join(format!("{NAME}.plist")))
}

/// The launchd domain: the system's as root, else the user's login session.
fn domain() -> String {
    // SAFETY: no preconditions.
    #[expect(unsafe_code, reason = "getuid has no preconditions")]
    let uid = unsafe { libc::getuid() };
    if sys::is_root() { "system".into() } else { format!("gui/{uid}") }
}

/// The app that runs the server, so that Login Items shows its name and icon and Spotlight finds it, and
/// that opens the web UI from Finder.
fn app() -> Result<PathBuf> {
    Ok(if sys::is_root() { PathBuf::from("/Applications") } else { etcetera::home_dir()?.join("Applications") }
        .join(format!("{APP_NAME}.app")))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn property_list(dict: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
{dict}</dict>
</plist>
"#
    )
}

/// The app, registered so that Finder and the Dock show its icon.
fn write_app(app: &Path, exe: &Path, version: &str) -> Result<PathBuf> {
    let bundled = bundle(app, exe, version)?;
    // Finder and the Dock keep showing the icon they cached until the app is registered again.
    register(app)?;
    Ok(bundled)
}

/// Registers `app` with Launch Services again, as `lsregister -f` does.
#[expect(unsafe_code, reason = "Launch Services and CoreFoundation are C functions")]
fn register(app: &Path) -> Result<()> {
    use std::{ffi::c_void, os::unix::ffi::OsStrExt};
    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSRegisterURL(url: *const c_void, update: bool) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(
            allocator: *const c_void,
            path: *const u8,
            len: isize,
            directory: bool,
        ) -> *const c_void;
        fn CFRelease(object: *const c_void);
    }
    let path = app.as_os_str().as_bytes();
    // SAFETY: a path that lives through the call, and a URL released once.
    let status = unsafe {
        let url = CFURLCreateFromFileSystemRepresentation(
            std::ptr::null(),
            path.as_ptr(),
            isize::try_from(path.len())?,
            true,
        );
        if url.is_null() {
            return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput).into());
        }
        let status = LSRegisterURL(url, true);
        CFRelease(url);
        status
    };
    match status {
        0 => Ok(()),
        code => Err(std::io::Error::other(format!("{code}")).into()),
    }
}

/// The app, with a copy of this executable, signed as itself rather than by the linker's anonymous
/// signature. A copy rather than a link, which macOS would resolve to the executable outside the app.
pub(super) fn bundle(app: &Path, exe: &Path, version: &str) -> Result<PathBuf> {
    let contents = app.join("Contents");
    // Named as the app: Login Items and Activity Monitor show it.
    let bundled = contents.join("MacOS").join(APP_NAME);
    for dir in ["MacOS", "Resources"] {
        std::fs::create_dir_all(contents.join(dir))?;
    }
    if exe != bundled {
        // Replaced whole, so that a copy that runs keeps its file. Removed first, as a disk that ignores
        // case would keep the old name's case.
        let partial = bundled.with_extension("partial");
        std::fs::copy(exe, &partial)?;
        _ = std::fs::remove_file(&bundled);
        std::fs::rename(&partial, &bundled)?;
        // A package manager's executable, which it updates, is copied again at start when it no longer is
        // the one copied: see `refresh_app`. Named with its SHA-256, as signing the app changes the copy.
        let source = crate::install::app::source_file(&contents);
        if crate::install::managed(exe) {
            let copied = sys::sha256(std::fs::read(exe)?);
            std::fs::write(&source, format!("{}\n{copied}\n", exe.display()))?;
        } else {
            _ = std::fs::remove_file(&source);
        }
    }
    std::fs::write(
        contents.join("Resources").join(format!("{NAME}.icns")),
        include_bytes!("../../../assets/images/logo.icns"),
    )?;
    let info = format!(
        "  <key>CFBundleIdentifier</key><string>{BUNDLE_ID}</string>
  <key>CFBundleName</key><string>{APP_NAME}</string>
  <key>CFBundleExecutable</key><string>{APP_NAME}</string>
  <key>CFBundleIconFile</key><string>{NAME}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>LSUIElement</key><true/>
"
    );
    std::fs::write(contents.join("Info.plist"), property_list(&info))?;
    run("codesign", &["--force", "--sign", "-", &app.to_string_lossy()])?;
    // Kept outside the signature, which a later one would not take.
    if let Some(keywords) = bplist(&env!("KEYWORDS").split(';').collect::<Vec<_>>()) {
        set_attribute(app, c"com.apple.metadata:kMDItemKeywords", &keywords)?;
    }
    Ok(bundled)
}

/// Sets the extended attribute `name` of `path` to `value`, as `xattr -w` does.
#[expect(unsafe_code, reason = "setxattr reads the NUL-terminated path and name, and the value, it is given")]
fn set_attribute(path: &Path, name: &std::ffi::CStr, value: &[u8]) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: a path, a name and a value that live through the call.
    let set = unsafe { libc::setxattr(path.as_ptr(), name.as_ptr(), value.as_ptr().cast(), value.len(), 0, 0) };
    if set == 0 { Ok(()) } else { Err(std::io::Error::last_os_error().into()) }
}

/// The package manager's executable when it changed since the app that holds `exe` copied it: not when the
/// app's signature changed the copy, else each start would copy it again.
#[cfg(any(feature = "self-update", test))]
pub(super) fn stale(exe: &Path) -> Option<PathBuf> {
    let (source, copied) = crate::install::app::source(exe)?;
    let new = std::fs::read(&source).ok()?;
    (sys::sha256(new) != copied).then_some(source)
}

/// The package manager's executable, copied into the app that holds this one when it changed since the
/// last copy: its version, or `None` when it is the same, or gone.
#[cfg(feature = "self-update")]
#[expect(clippy::disallowed_methods, reason = "asks the package manager's rusticlone its version")]
pub(super) fn refresh_app() -> Result<Option<String>> {
    let exe = std::env::current_exe()?;
    let Some(source) = stale(&exe) else { return Ok(None) };
    let out = std::process::Command::new(&source).arg("--version").output()?;
    let version = String::from_utf8_lossy(&out.stdout).split_whitespace().last().unwrap_or_default().to_owned();
    let app = exe.parent().and_then(Path::parent).and_then(Path::parent).unwrap_or(&exe);
    write_app(app, &source, &version)?;
    Ok(Some(version))
}

/// The app that holds this executable, written again with `version`.
#[cfg(feature = "self-update")]
pub(super) fn rewrite_app(version: &str) -> Result<()> {
    let exe = std::env::current_exe()?;
    // The executable, in `Contents/MacOS` of the app.
    match exe.parent().and_then(Path::parent).and_then(Path::parent) {
        Some(app) if exe.ends_with(Path::new("Contents").join("MacOS").join(APP_NAME)) => {
            write_app(app, &exe, version).map(drop)
        }
        _ => Ok(()),
    }
}

/// Whether the plist is installed and the user switched it off. macOS 13 introduced the switch.
#[expect(unsafe_code, reason = "asks Service Management about a plist's path")]
pub(super) fn blocked() -> bool {
    use objc2_foundation::{NSString, NSURL};
    use objc2_service_management::{SMAppService, SMAppServiceStatus};
    let Ok(file) = plist() else { return false };
    if !file.exists() || objc2::runtime::AnyClass::get(c"SMAppService").is_none() {
        return false;
    }
    let url = NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy()));
    // SAFETY: a file URL; Apple states no other requirement.
    unsafe { SMAppService::statusForLegacyURL(&url) == SMAppServiceStatus::RequiresApproval }
}

/// Started now and at load, restarted after a failure, at low priority, as the systemd unit.
pub(super) fn install(_listen: &str, exe: &Path, serve: &[String]) -> Result<PathBuf> {
    let (file, dir) = (plist()?, crate::profile::own_dir().ok_or(NoProfileDir {})?);
    // rustic's profile directory, so that the server reads no profiles from wherever it started.
    std::fs::create_dir_all(&dir)?;
    // Stopped before its app is replaced. Loaded already when installed before.
    _ = run("launchctl", &["bootout", &format!("{}/{NAME}", domain())]);
    let exe = write_app(&app()?, exe, env!("CARGO_PKG_VERSION"))?;
    let text = property_list(&format!(
        "  <key>Label</key><string>{NAME}</string>
  <key>ProgramArguments</key><array><string>{}</string>{}</array>
  <key>EnvironmentVariables</key><dict><key>PATH</key><string>{}</string></dict>
  <key>WorkingDirectory</key><string>{}</string>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>Nice</key><integer>10</integer>
  <key>LowPriorityIO</key><true/>
",
        escape(&exe.to_string_lossy()),
        serve.iter().map(|a| format!("<string>{}</string>", escape(a))).collect::<String>(),
        escape(&path_var()),
        escape(&dir.to_string_lossy())
    ));
    std::fs::create_dir_all(file.parent().unwrap_or(&file))?;
    std::fs::write(&file, text)?;
    run("launchctl", &["bootstrap", &domain(), &file.to_string_lossy()])?;
    Ok(file)
}

/// Whether `launchctl disable` keeps launchd from loading the plist at login. Older macOS says `true`.
fn disabled() -> bool {
    let listed = std::process::Command::new("launchctl").args(["print-disabled", &domain()]).output();
    let entry = format!("\"{NAME}\" =>");
    listed.is_ok_and(|out| {
        String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().strip_prefix(&entry)).any(|state| {
            let state = state.trim();
            state == "disabled" || state == "true"
        })
    })
}

pub(super) fn at_login() -> Option<bool> {
    plist().ok().filter(|f| f.exists())?;
    Some(!disabled() && !blocked())
}

/// Switched in launchd. Switched off in Login Items, the server waits for the user there, so it opens.
pub(super) fn set_at_login(on: bool) -> Result<()> {
    run("launchctl", &[if on { "enable" } else { "disable" }, &format!("{}/{NAME}", domain())])?;
    if on && blocked() {
        // SAFETY: Apple states no requirement.
        #[expect(unsafe_code, reason = "opens System Settings through Service Management")]
        unsafe {
            objc2_service_management::SMAppService::openSystemSettingsLoginItems();
        }
    }
    Ok(())
}

/// Asks launchd to start the installed server, as installed, unless it runs; false if not installed, or
/// kept from starting at login, when launchd may not have loaded it.
pub(super) fn start() -> Result<bool> {
    if !plist()?.exists() || disabled() {
        return Ok(false);
    }
    run("launchctl", &["kickstart", &format!("{}/{NAME}", domain())])?;
    Ok(true)
}

/// `None` if not installed.
pub(super) fn uninstall() -> Result<Option<Removed>> {
    let file = plist()?;
    if !file.exists() {
        return Ok(None);
    }
    // Not loaded if it stopped, or after a reboot that failed to load it.
    _ = run("launchctl", &["bootout", &format!("{}/{NAME}", domain())]);
    std::fs::remove_file(&file)?;
    _ = std::fs::remove_dir_all(app()?);
    Ok(Some(Removed { file: file.display().to_string(), running: false }))
}

/// rustic's profile folder is unknown, so the server would read no profiles.
#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "autostart-error-no-profile-dir")]
pub(crate) struct NoProfileDir {}

/// Whether this is the app opened from Finder or Spotlight, which passes no arguments.
pub(super) fn opened_as_app() -> bool {
    let inside = format!("Contents/MacOS/{APP_NAME}");
    std::env::args_os().len() == 1 && std::env::current_exe().is_ok_and(|exe| exe.ends_with(inside))
}

/// launchd runs it.
#[expect(clippy::unnecessary_wraps, reason = "Windows's stops a running server, which may fail")]
pub(super) fn stop_running(_: &Path) -> Result<()> {
    Ok(())
}

/// Linux's alone: a user without a desktop.
pub(super) fn headless() -> bool {
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_keywords_fit_a_short_property_list() {
        let keywords: Vec<&str> = env!("KEYWORDS").split(';').collect();
        let plist = super::bplist(&keywords).expect("Cargo.toml's keywords are few, short and ASCII");
        assert!(
            plist.starts_with(b"bplist00")
                && keywords.iter().all(|k| plist.windows(k.len()).any(|w| w == k.as_bytes()))
        );
    }

    /// The app copies Homebrew's executable and signs the copy, which changes it: the app is not stale then, else
    /// each start would copy it again and restart. It is once Homebrew upgrades its executable.
    #[test]
    fn the_app_copies_homebrews_executable_again_only_once_it_changed() {
        let dir = std::env::temp_dir().join(format!("{}-brew-app-{}", crate::NAME, std::process::id()));
        let (app, cellar) = (
            dir.join(format!("{}.app", crate::APP_NAME)),
            dir.join("Cellar").join("rusticlone").join("2.0.1").join("bin"),
        );
        std::fs::create_dir_all(&cellar).unwrap();
        let source = cellar.join(crate::NAME);
        std::fs::copy("/usr/bin/true", &source).unwrap();
        let exe = super::bundle(&app, &source, "2.0.1").unwrap();
        let signed_copy_differs = std::fs::read(&exe).unwrap() != std::fs::read(&source).unwrap();
        let after_install = super::stale(&exe);
        std::fs::copy("/usr/bin/false", &source).unwrap();
        let after_upgrade = super::stale(&exe);
        std::fs::remove_dir_all(dir).unwrap();
        assert!(signed_copy_differs, "codesign no longer changes the copy: the test proves less");
        assert_eq!(after_install, None);
        assert_eq!(after_upgrade, Some(source));
    }

    /// The app keeps its Spotlight keywords in the attribute that `xattr` reads.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "reads the attribute with xattr, which the code no longer runs")]
    fn the_app_has_its_keywords() {
        let app = std::env::temp_dir().join(format!("{}-keywords-{}.app", crate::NAME, std::process::id()));
        super::bundle(&app, &std::path::Path::new("/usr").join("bin").join("true"), "2.0.1").unwrap();
        let read =
            std::process::Command::new("xattr").args(["-px", "com.apple.metadata:kMDItemKeywords"]).arg(&app).output();
        let hex: String = String::from_utf8(read.unwrap().stdout).unwrap().split_whitespace().collect();
        std::fs::remove_dir_all(&app).unwrap();
        let keywords = super::bplist(&env!("KEYWORDS").split(';').collect::<Vec<_>>()).unwrap();
        assert_eq!(hex.to_lowercase(), keywords.iter().map(|b| format!("{b:02x}")).collect::<String>());
    }

    /// An update replaces the executable inside the app, which breaks its signature: written again, the app
    /// shows the new version and its signature holds.
    #[test]
    fn the_app_is_signed_again_around_a_new_executable() {
        let dir = std::env::temp_dir().join(format!("{}-app-{}", crate::NAME, std::process::id()));
        let app = dir.join(format!("{}.app", crate::APP_NAME));
        let signed = || {
            let verify = std::process::Command::new("codesign").args(["--verify", "--strict"]).arg(&app).output();
            verify.unwrap().status.success()
        };
        let exe = super::bundle(&app, &std::path::Path::new("/usr").join("bin").join("true"), "1.0.0").unwrap();
        assert!(signed());
        std::fs::copy("/usr/bin/false", &exe).unwrap();
        assert!(!signed());
        super::bundle(&app, &exe, "1.0.1").unwrap();
        let info = std::fs::read_to_string(app.join("Contents").join("Info.plist")).unwrap();
        assert!(signed() && info.contains("<string>1.0.1</string>"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
