//! `install`: the server starts with the computer as root or headless on Linux, else at login, through the system's
//! own service manager, and starts now. It runs this executable with the current `PATH`, so that it
//! finds the same rustic.

#![expect(clippy::disallowed_methods, reason = "writes the service files and runs the service manager")]

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::log;

mod place;

#[derive(Debug, Serialize, crate::i18n::Fluent)]
#[fluent(prefix = "autostart")]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum Logged {
    /// As root, an administrator, the system service, or a headless Linux user.
    Installed {
        file: String,
    },
    InstalledAtLogin {
        file: String,
    },
    Uninstalled {
        file: String,
    },
    /// The server started at login from the `Run` key has no service to stop it: it runs until logout.
    #[cfg(windows)]
    UninstalledRunning {
        file: String,
    },
    #[cfg(feature = "self-update")]
    #[fluent(key = crate::settings::AUTO_UPDATE.key)]
    AutoUpdate {
        file: String,
    },
    NotInstalled,
    Deleted {
        dir: String,
    },
    /// The macOS app copied the executable that the package manager updated.
    #[cfg(all(target_vendor = "apple", feature = "self-update"))]
    AppRefreshed {
        version: String,
    },
}

/// None on Windows, where the registry's errors are I/O errors.
#[cfg(not(windows))]
#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "autostart-error")]
pub(crate) enum Error {
    #[cfg(unix)]
    CannotRun { program: String, source: std::io::Error },
    #[cfg(unix)]
    Failed { command: String, error: String },
    #[cfg(target_vendor = "apple")]
    NoProfileDir,
    #[cfg(not(any(target_os = "linux", target_vendor = "apple", windows)))]
    Unsupported,
}

/// The user switched the server off in macOS's Login Items, so launchd no longer starts it.
#[derive(Debug, Serialize, crate::i18n::Fluent)]
#[serde(tag = "kind", rename = "login-item-off")]
#[fluent(prefix = "autostart-blocked", app = crate::APP_NAME)]
pub(crate) struct Blocked {}

log::level!(Success: Logged);
log::level!(Warning: Blocked);

/// Whether the server is installed to start at login, and the user switched it off.
pub(crate) fn blocked() -> Option<Blocked> {
    #[cfg(target_vendor = "apple")]
    return imp::blocked().then_some(Blocked {});
    #[cfg(not(target_vendor = "apple"))]
    None
}

/// Runs a service manager's command, failing with its output.
#[cfg(unix)]
fn run(program: &str, args: &[&str]) -> Result<()> {
    let out = (std::process::Command::new(program).args(args).output())
        .map_err(|source| Error::CannotRun { program: program.to_owned(), source })?;
    if !out.status.success() {
        let (command, output) = (format!("{program} {}", args.join(" ")), String::from_utf8_lossy(&out.stderr));
        return Err(Error::Failed { command, error: output.trim().to_owned() }.into());
    }
    Ok(())
}

/// Starts the installed or packaged server, rather than one without its options, and waits until it answers on
/// `listen`: false without one, or if it does not answer within a minute, which an update at its start can take.
/// The service manager then supervises it, unlike a server that `serve --open` starts.
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
pub(crate) fn start_installed(listen: &str) -> Result<bool> {
    Ok(imp::start()? && answers_soon(listen))
}

/// Whether the server answers on `listen` within a minute, which an update at its start can take.
pub(crate) fn answers_soon(listen: &str) -> bool {
    const POLL: std::time::Duration = std::time::Duration::from_millis(200);
    let deadline = std::time::Instant::now() + std::time::Duration::from_mins(1);
    while crate::health::check(listen).is_err() {
        if std::time::Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
    true
}

/// The package manager's executable that the macOS app holding `exe` is a copy of: the package manager
/// updates that one.
#[cfg(target_vendor = "apple")]
pub(crate) fn app_source(exe: &Path) -> Option<PathBuf> {
    imp::app_source(exe)
}

/// Copies the package manager's executable into the macOS app again when it updated it, and says so.
/// Whether it did: the app then starts again.
#[cfg(all(target_vendor = "apple", feature = "self-update"))]
pub(crate) fn refresh_app() -> Result<bool> {
    let Some(version) = imp::refresh_app()? else { return Ok(false) };
    log::event(None, &Logged::AppRefreshed { version });
    Ok(true)
}

/// Writes the macOS app again around this executable, which an update replaced with `version`: its version,
/// and a signature that covers the new executable. Nothing for an executable outside the app.
#[cfg(all(target_vendor = "apple", feature = "self-update"))]
pub(crate) fn rewrite_app(version: &str) -> Result<()> {
    imp::rewrite_app(version)
}

/// Whether this is the macOS app opened from Finder or Spotlight, which passes no arguments.
#[cfg(target_vendor = "apple")]
pub(crate) fn opened_as_app() -> bool {
    let inside = format!("Contents/MacOS/{}", crate::APP_NAME);
    std::env::args_os().len() == 1 && std::env::current_exe().is_ok_and(|exe| exe.ends_with(inside))
}

#[cfg(windows)]
pub(crate) use imp::stop_event;

#[cfg(windows)]
fn exe() -> Result<String> {
    Ok(std::env::current_exe()?.to_string_lossy().into_owned())
}

#[cfg(unix)]
fn path_var() -> String {
    std::env::var("PATH").unwrap_or_default()
}

/// The systemd unit of the server, which packages install too. In English, as packages carry it.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn unit() -> String {
    let description = crate::i18n::Language::english().scope(|| crate::i18n::t!("autostart-unit-description"));
    format!(
        r"[Unit]
Description={description}
Wants=network-online.target
After=network-online.target

[Service]
Type=notify-reload
ExecStart=%N serve
WorkingDirectory=-%E/rustic
Restart=on-failure
Nice=10
IOSchedulingClass=idle
UMask=0077
NoNewPrivileges=yes
PrivateTmp=yes
ProtectProc=invisible
ProtectClock=yes
ProtectHostname=yes
ProtectKernelLogs=yes
ProtectKernelModules=yes
ProtectKernelTunables=yes
ProtectControlGroups=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
RestrictRealtime=yes
RestrictNamespaces=yes
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK
SystemCallArchitectures=native
SystemCallFilter=@system-service @mount
SystemCallErrorNumber=EPERM
CapabilityBoundingSet=CAP_CHOWN CAP_DAC_OVERRIDE CAP_DAC_READ_SEARCH CAP_FOWNER CAP_FSETID CAP_MKNOD CAP_SETFCAP CAP_SYS_ADMIN

[Install]
WantedBy=multi-user.target default.target
"
    )
}

/// Message `id` in each translation that has it.
#[cfg(any(target_os = "linux", test))]
fn translated(id: &str) -> Vec<(String, String)> {
    #[cfg(feature = "server")]
    return crate::i18n::translations(id);
    #[cfg(not(feature = "server"))]
    Vec::new()
}

/// An application that opens the web UI logged in, by running `exe`, quoted as desktop entries require.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn desktop_entry(exe: &str) -> String {
    let quoted: String =
        exe.chars().flat_map(|c| ['\\'].into_iter().filter(move |_| "\"`$\\".contains(c)).chain([c])).collect();
    // Backslashes are escaped again in any value, and `%` starts a field code.
    let exe = quoted.replace('\\', "\\\\").replace('%', "%%");
    format!(
        r#"[Desktop Entry]
Type=Application
Name={APP_NAME}
Comment={DESCRIPTION}
{COMMENTS}Exec="{exe}" serve --open
Icon={NAME}
Terminal=false
Categories=Utility;Archiving;
Keywords={KEYWORDS};
"#,
        NAME = crate::NAME,
        APP_NAME = crate::APP_NAME,
        DESCRIPTION = env!("CARGO_PKG_DESCRIPTION"),
        // The description in each other language that has it, as desktops pick it: `Comment[it]=`.
        COMMENTS = translated("app-description")
            .iter()
            .map(|(language, text)| format!("Comment[{}]={text}\n", language.replace('-', "_")))
            .collect::<String>(),
        KEYWORDS = env!("KEYWORDS")
    )
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{Logged, Path, PathBuf, Result, path_var, run};
    use crate::{NAME, sys};

    const ICON: &[u8] = include_bytes!("../../assets/images/logo.svg");

    /// The desktop entry and its icon, for a user: root has no desktop.
    fn desktop_files() -> Result<Option<[PathBuf; 2]>> {
        if sys::is_root() {
            return Ok(None);
        }
        let data = etcetera::choose_base_strategy().map(|b| etcetera::BaseStrategy::data_dir(&b))?;
        let entry = data.join(format!("applications/{NAME}.desktop"));
        Ok(Some([entry, data.join(format!("icons/hicolor/scalable/apps/{NAME}.svg"))]))
    }

    fn systemctl(args: &[&str]) -> Result<()> {
        let user = (!sys::is_root()).then_some("--user");
        run("systemctl", &user.into_iter().chain(args.iter().copied()).collect::<Vec<_>>())
    }

    fn dir() -> Result<PathBuf> {
        Ok(if sys::is_root() {
            "/etc/systemd/system".into()
        } else {
            etcetera::choose_base_strategy().map(|b| etcetera::BaseStrategy::config_dir(&b))?.join("systemd/user")
        })
    }

    /// A user without a graphical session, as on a server over SSH, whose services systemd would stop at logout.
    pub(super) fn headless() -> bool {
        !sys::is_root() && ["DISPLAY", "WAYLAND_DISPLAY"].iter().all(|v| std::env::var_os(v).is_none())
    }

    /// Writes the unit unless a package installed it, and a drop-in with this executable and `PATH`. Restarted,
    /// so that a running server takes them. Headless, systemd keeps the user's services running without a login.
    pub(super) fn install(_listen: &str, exe: &Path, serve: &[String]) -> Result<PathBuf> {
        if headless() {
            run("loginctl", &["enable-linger"])?;
        }
        let (dir, service) = (dir()?, format!("{NAME}.service"));
        let unit = dir.join(&service);
        if !unit.exists() && !known() {
            std::fs::create_dir_all(&dir)?;
            std::fs::write(&unit, super::unit())?;
        }
        let drop_in = dir.join(format!("{service}.d"));
        std::fs::create_dir_all(&drop_in)?;
        // `%` starts a specifier.
        let exe = exe.display().to_string();
        let (escaped, path) = (exe.replace('%', "%%"), path_var().replace('%', "%%"));
        let serve = serve.join(" ");
        let conf = format!("[Service]\nExecStart=\nExecStart=\"{escaped}\" {serve}\nEnvironment=\"PATH={path}\"\n");
        std::fs::write(drop_in.join("install.conf"), conf)?;
        systemctl(&["daemon-reload"])?;
        systemctl(&["enable", &service])?;
        systemctl(&["restart", &service])?;
        if let Some([entry, icon]) = desktop_files()? {
            for (path, content) in [(&entry, super::desktop_entry(&exe).into_bytes()), (&icon, ICON.to_vec())] {
                std::fs::create_dir_all(path.parent().unwrap_or(path))?;
                std::fs::write(path, content)?;
            }
        }
        Ok(drop_in)
    }

    /// Whether systemd has the unit, from `install` or a package: a packaged one serves without `install`.
    fn known() -> bool {
        systemctl(&["cat", &format!("{NAME}.service")]).is_ok()
    }

    pub(super) fn at_login() -> Option<bool> {
        known().then(|| systemctl(&["is-enabled", "--quiet", &format!("{NAME}.service")]).is_ok())
    }

    pub(super) fn set_at_login(on: bool) -> Result<()> {
        systemctl(&[if on { "enable" } else { "disable" }, &format!("{NAME}.service")])
    }

    /// Asks systemd to start the server unless it runs; false without its unit.
    pub(super) fn start() -> Result<bool> {
        if !known() {
            return Ok(false);
        }
        systemctl(&["start", &format!("{NAME}.service")])?;
        Ok(true)
    }

    /// `None` if not installed.
    pub(super) fn uninstall() -> Result<Option<Logged>> {
        let (dir, service) = (dir()?, format!("{NAME}.service"));
        let drop_in = dir.join(format!("{service}.d"));
        if !drop_in.join("install.conf").exists() {
            return Ok(None);
        }
        systemctl(&["disable", "--now", &service])?;
        std::fs::remove_file(drop_in.join("install.conf"))?;
        _ = std::fs::remove_dir(&drop_in);
        // A packaged unit stays.
        if std::fs::read_to_string(dir.join(&service)).is_ok_and(|u| u == super::unit()) {
            std::fs::remove_file(dir.join(&service))?;
        }
        systemctl(&["daemon-reload"])?;
        for file in desktop_files()?.into_iter().flatten() {
            _ = std::fs::remove_file(file);
        }
        Ok(Some(Logged::Uninstalled { file: drop_in.display().to_string() }))
    }
}

#[cfg(target_vendor = "apple")]
mod imp {
    use super::{Error, Logged, Path, PathBuf, Result, path_var, run};
    use crate::{APP_NAME, NAME, sys};

    /// What macOS keys the user's approval and the app's permissions on: it never changes.
    const BUNDLE_ID: &str = "io.github.alphajack.rusticlone";
    const LSREGISTER: &str =
        "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
    /// System Settings, General, Login Items & Extensions.
    const LOGIN_ITEMS: &str = "x-apple.systempreferences:com.apple.LoginItems-Settings.extension";

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
            PathBuf::from("/Library/LaunchDaemons")
        } else {
            etcetera::home_dir()?.join("Library/LaunchAgents")
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
        run(LSREGISTER, &["-f", &app.to_string_lossy()])?;
        Ok(bundled)
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
            // A package manager's executable, which it updates, is copied again at start: see `refresh_app`.
            let source = source_file(&contents);
            if crate::install::managed(exe) {
                std::fs::write(&source, exe.as_os_str().as_encoded_bytes())?;
            } else {
                _ = std::fs::remove_file(&source);
            }
        }
        std::fs::write(
            contents.join("Resources").join(format!("{NAME}.icns")),
            include_bytes!("../../assets/images/logo.icns"),
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
            let hex: String = keywords.iter().map(|b| format!("{b:02x}")).collect();
            run("xattr", &["-wx", "com.apple.metadata:kMDItemKeywords", &hex, &app.to_string_lossy()])?;
        }
        Ok(bundled)
    }

    /// Where the app names the package manager's executable that it is a copy of.
    fn source_file(contents: &Path) -> PathBuf {
        contents.join("Resources").join("source")
    }

    pub(super) fn app_source(exe: &Path) -> Option<PathBuf> {
        if !exe.ends_with(Path::new("Contents/MacOS").join(APP_NAME)) {
            return None;
        }
        let text = std::fs::read_to_string(source_file(exe.parent()?.parent()?)).ok()?;
        Some(PathBuf::from(text.trim_end()))
    }

    /// The package manager's executable, copied into the app that holds this one when it differs: its
    /// version, or `None` when it is the same, or gone.
    #[cfg(feature = "self-update")]
    #[expect(clippy::disallowed_methods, reason = "asks the package manager's rusticlone its version")]
    pub(super) fn refresh_app() -> Result<Option<String>> {
        let exe = std::env::current_exe()?;
        let Some(source) = app_source(&exe) else { return Ok(None) };
        let Ok(new) = std::fs::read(&source) else { return Ok(None) };
        if new == std::fs::read(&exe)? {
            return Ok(None);
        }
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
            Some(app) if exe.ends_with(Path::new("Contents/MacOS").join(APP_NAME)) => {
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
        let (file, dir) = (plist()?, crate::profile::own_dir().ok_or(Error::NoProfileDir)?);
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
  <key>ProcessType</key><string>Background</string>
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
            run("open", &[LOGIN_ITEMS])?;
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
    pub(super) fn uninstall() -> Result<Option<Logged>> {
        let file = plist()?;
        if !file.exists() {
            return Ok(None);
        }
        // Not loaded if it stopped, or after a reboot that failed to load it.
        _ = run("launchctl", &["bootout", &format!("{}/{NAME}", domain())]);
        std::fs::remove_file(&file)?;
        _ = std::fs::remove_dir_all(app()?);
        Ok(Some(Logged::Uninstalled { file: file.display().to_string() }))
    }
}

/// From an elevated shell, a system service that starts with the computer, restarted after a failure; else
/// the user's `Run` key, so that the server starts in the user's session at login, reads the user's profiles,
/// and shows its tray icon and desktop notifications, and a Start menu entry that opens the web UI.
#[cfg(windows)]
mod imp {
    use std::time::Duration;

    use windows_service::{
        service::{
            Service, ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl, ServiceFailureActions,
            ServiceFailureResetPeriod, ServiceInfo, ServiceStartType, ServiceState, ServiceType,
        },
        service_manager::{ServiceManager, ServiceManagerAccess},
    };
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SERVICE_DOES_NOT_EXIST, ERROR_SUCCESS},
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_MULTI_SZ, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW,
        },
    };

    use super::{Logged, Path, PathBuf, Result, exe};
    use crate::{APP_NAME, NAME, settings, sys};

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const SERVICES: &str = r"SYSTEM\CurrentControlSet\Services";
    /// After the third failure within the reset period, restarts wait the longest.
    const RESTARTS: [Duration; 3] = [Duration::from_secs(3), Duration::from_secs(30), Duration::from_mins(10)];
    const RESET: Duration = Duration::from_mins(15);
    /// How often a stopping service is asked whether it stopped, and for how long.
    const POLL: Duration = Duration::from_millis(200);
    const STOP_TIMEOUT: Duration = Duration::from_secs(30);
    /// Windows 11 24H2, the first to read the console policy of a manifest.
    const CONSOLE_POLICY_BUILD: u32 = 26100;
    /// Longer than any path of an executable that Windows runs.
    const MAX_PATH: usize = 1024;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }

    fn os_error(status: u32) -> std::io::Error {
        std::io::Error::from_raw_os_error(status.cast_signed())
    }

    /// Sets `value` of `key`, as `kind`, from its UTF-16 data.
    #[expect(unsafe_code, reason = "writes a registry value from NUL-terminated strings")]
    fn set(root: HKEY, key: &str, value: &str, kind: u32, data: &[u16]) -> Result<()> {
        let size = u32::try_from(size_of_val(data))?;
        // SAFETY: NUL-terminated strings, and the data's size in bytes.
        let status = unsafe {
            RegSetKeyValueW(root, wide(key).as_ptr(), wide(value).as_ptr(), kind, data.as_ptr().cast(), size)
        };
        if status != ERROR_SUCCESS {
            return Err(os_error(status).into());
        }
        Ok(())
    }

    fn manager(access: ServiceManagerAccess) -> Result<ServiceManager> {
        Ok(ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | access)?)
    }

    fn not_installed(error: &windows_service::Error) -> bool {
        matches!(error, windows_service::Error::Winapi(e)
            if e.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST.cast_signed()))
    }

    /// Asks the service to stop, and waits until it has: runs get [`crate::server`]'s time to record that.
    fn stop(service: &Service) -> Result<()> {
        if service.query_status()?.current_state == ServiceState::Stopped {
            return Ok(());
        }
        service.stop()?;
        let deadline = std::time::Instant::now() + STOP_TIMEOUT;
        while service.query_status()?.current_state != ServiceState::Stopped && std::time::Instant::now() < deadline {
            std::thread::sleep(POLL);
        }
        Ok(())
    }

    fn install_service(exe: &Path, serve: &[String]) -> Result<PathBuf> {
        let info = ServiceInfo {
            name: NAME.into(),
            display_name: APP_NAME.into(),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: exe.to_owned(),
            launch_arguments: serve.iter().map(Into::into).collect(),
            dependencies: Vec::new(),
            // LocalSystem.
            account_name: None,
            account_password: None,
        };
        let access =
            ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::STOP | ServiceAccess::CHANGE_CONFIG;
        let manager = manager(ServiceManagerAccess::CREATE_SERVICE)?;
        // Installed before: updated.
        let service = manager.create_service(&info, access).or_else(|_| {
            let service = manager.open_service(NAME, access)?;
            service.change_config(&info)?;
            Ok::<_, windows_service::Error>(service)
        })?;
        let actions = RESTARTS.map(|delay| ServiceAction { action_type: ServiceActionType::Restart, delay });
        service.update_failure_actions(ServiceFailureActions {
            reset_period: ServiceFailureResetPeriod::After(RESET),
            reboot_msg: None,
            command: None,
            actions: Some(actions.into()),
        })?;
        // Also after an exit with an error, not only after a crash.
        service.set_failure_actions_on_non_crash_failures(true)?;
        // This `PATH` and rustic, so that it runs the same rustic, even one downloaded into this user's
        // directory rather than the system account's.
        let rustic = crate::rustic::bin();
        let environment = [format!("PATH={}", std::env::var("PATH").unwrap_or_default())]
            .into_iter()
            .chain(rustic.is_absolute().then(|| format!("{}={}", settings::RUSTIC.key, rustic.display())));
        let data: Vec<u16> =
            environment.flat_map(|v| v.encode_utf16().chain([0]).collect::<Vec<_>>()).chain([0]).collect();
        let key = format!(r"{SERVICES}\{NAME}");
        set(HKEY_LOCAL_MACHINE, &key, "Environment", REG_MULTI_SZ, &data)?;
        // Restarted, so that a running server takes the new executable.
        stop(&service)?;
        service.start::<&str>(&[])?;
        Ok(PathBuf::from(format!(r"HKEY_LOCAL_MACHINE\{key}")))
    }

    /// Stops the running server, so that `exe` can be replaced: the service, or the server in this session.
    /// Then waits, at most [`STOP_TIMEOUT`], until every process that runs `exe` has exited, which is after
    /// the server answers no more and its threads end.
    pub(super) fn stop_running(exe: &Path) -> Result<()> {
        if sys::is_root() {
            let access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP;
            match manager(ServiceManagerAccess::empty())?.open_service(NAME, access) {
                Err(e) if not_installed(&e) => {}
                service => stop(&service?)?,
            }
        } else {
            ask_to_stop();
        }
        wait_for(exe);
        Ok(())
    }

    /// Asks the server in this session, if any, to stop.
    #[expect(unsafe_code, reason = "opens a named event, sets it and closes it")]
    fn ask_to_stop() {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{EVENT_MODIFY_STATE, OpenEventW, SetEvent},
        };
        // SAFETY: a NUL-terminated name; the handle is closed once set.
        unsafe {
            let event = OpenEventW(EVENT_MODIFY_STATE, 0, stop_event().as_ptr());
            if !event.is_null() {
                SetEvent(event);
                CloseHandle(event);
            }
        }
    }

    /// The event that asks the server in this session to stop, as the service manager asks the service, so
    /// that `install` can replace its executable.
    pub(crate) fn stop_event() -> Vec<u16> {
        format!(r"Local\{NAME}-stop").encode_utf16().chain([0]).collect()
    }

    /// Waits until the processes that run `exe` have exited, at most [`STOP_TIMEOUT`].
    #[expect(unsafe_code, reason = "lists the processes, and waits on those that run `exe`")]
    fn wait_for(exe: &Path) {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
            System::{
                Diagnostics::ToolHelp::{
                    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
                },
                Threading::{
                    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
                    QueryFullProcessImageNameW, WaitForMultipleObjects,
                },
            },
        };
        let mut running = Vec::new();
        // SAFETY: a snapshot read entry by entry, each into a struct of the size it states; each process opened
        // is queried into a buffer of the size given, and closed unless kept to wait on.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = u32::try_from(size_of::<PROCESSENTRY32W>()).unwrap_or_default();
            let mut more = Process32FirstW(snapshot, &raw mut entry) != 0;
            while more {
                let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
                let process = OpenProcess(access, 0, entry.th32ProcessID);
                if !process.is_null() {
                    let mut name = [0u16; MAX_PATH];
                    let mut size = u32::try_from(name.len()).unwrap_or_default();
                    let named =
                        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, name.as_mut_ptr(), &raw mut size);
                    let image = String::from_utf16_lossy(name.get(..size as usize).unwrap_or_default());
                    if named != 0
                        && image.eq_ignore_ascii_case(&exe.to_string_lossy())
                        && entry.th32ProcessID != std::process::id()
                    {
                        running.push(process);
                    } else {
                        CloseHandle(process);
                    }
                }
                more = Process32NextW(snapshot, &raw mut entry) != 0;
            }
            CloseHandle(snapshot);
            if !running.is_empty() {
                let count = u32::try_from(running.len()).unwrap_or_default();
                let timeout = u32::try_from(STOP_TIMEOUT.as_millis()).unwrap_or(u32::MAX);
                WaitForMultipleObjects(count, running.as_ptr(), 1, timeout);
            }
            for process in running {
                CloseHandle(process);
            }
        }
    }

    fn uninstall_service() -> Result<Option<Logged>> {
        let access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE;
        let service = match manager(ServiceManagerAccess::empty())?.open_service(NAME, access) {
            Err(e) if not_installed(&e) => return Ok(None),
            service => service?,
        };
        stop(&service)?;
        service.delete()?;
        Ok(Some(Logged::Uninstalled { file: format!(r"HKEY_LOCAL_MACHINE\{SERVICES}\{NAME}") }))
    }

    fn run_key() -> String {
        format!(r"HKEY_CURRENT_USER\{RUN}\{NAME}")
    }

    /// The Start menu entry. Unlike a desktop entry or the macOS app, it has no keywords: Start menu search
    /// matches only its name.
    fn start_menu() -> Result<PathBuf> {
        use etcetera::BaseStrategy;
        let programs = etcetera::choose_base_strategy()?.config_dir().join(r"Microsoft\Windows\Start Menu\Programs");
        Ok(programs.join(format!("{APP_NAME}.lnk")))
    }

    /// What runs `exe` with `args` without a console window, so that none opens at login or from the Start
    /// menu: `exe` itself from Windows 11 24H2 on, which reads the console policy of its manifest, else Windows's
    /// console host, which gives it a console that has no window.
    fn headless(exe: &str, args: &str) -> (PathBuf, String) {
        if sys::windows_build() >= CONSOLE_POLICY_BUILD {
            return (PathBuf::from(exe), args.into());
        }
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        (PathBuf::from(root).join(r"System32\conhost.exe"), format!("--headless \"{exe}\" {args}"))
    }

    pub(super) fn install(listen: &str, exe: &Path, serve: &[String]) -> Result<PathBuf> {
        use std::os::windows::process::CommandExt;
        if sys::is_root() {
            return install_service(exe, serve);
        }
        let exe = exe.display().to_string();
        set_run(&exe, serve)?;
        let (host, args) = headless(&exe, "serve --open");
        let mut link = mslnk::ShellLink::new(host)?;
        link.set_arguments(Some(args));
        link.set_icon_location(Some(exe.clone()));
        link.set_name(Some(crate::i18n::t!("app-description")));
        link.create_lnk(start_menu()?)?;
        // Started now without a console, as at login, unless it runs already. Without this one's output
        // either, which would otherwise keep printing into the terminal after install returns.
        if crate::health::check(listen).is_err() {
            use std::process::Stdio;
            keep_own_handles();
            let flags = windows_sys::Win32::System::Threading::DETACHED_PROCESS;
            let mut server = std::process::Command::new(exe);
            server.args(serve).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            server.creation_flags(flags).spawn()?;
        }
        Ok(PathBuf::from(run_key()))
    }

    /// The `Run` key's value that starts `exe` with `serve` at login.
    fn set_run(exe: &str, serve: &[String]) -> Result<()> {
        let (host, args) = headless(exe, &serve.join(" "));
        set(HKEY_CURRENT_USER, RUN, NAME, REG_SZ, &wide(&format!("\"{}\" {args}", host.display())))
    }

    /// Installed with its Start menu entry; started at login by the `Run` key.
    #[expect(unsafe_code, reason = "reads a registry value named by NUL-terminated strings")]
    pub(super) fn at_login() -> Option<bool> {
        use windows_sys::Win32::System::Registry::{RRF_RT_REG_SZ, RegGetValueW};
        start_menu().ok().filter(|link| link.exists())?;
        // SAFETY: NUL-terminated strings, and no buffer: it only asks whether the value exists.
        let read = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                wide(RUN).as_ptr(),
                wide(NAME).as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        Some(read == ERROR_SUCCESS)
    }

    #[expect(unsafe_code, reason = "deletes a registry value named by NUL-terminated strings")]
    pub(super) fn set_at_login(on: bool) -> Result<()> {
        if on {
            return set_run(&exe()?, &super::serve_args());
        }
        // SAFETY: NUL-terminated strings.
        match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(RUN).as_ptr(), wide(NAME).as_ptr()) } {
            ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
            status => Err(os_error(status).into()),
        }
    }

    /// Makes this process's standard input, output and error not inheritable. Rust lets a child inherit every
    /// inheritable handle, and the server would hold the pipe of a script that reads install's output, which
    /// then waits until the server stops.
    #[expect(unsafe_code, reason = "changes a flag of this process's own standard handles")]
    fn keep_own_handles() {
        use windows_sys::Win32::{
            Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation},
            System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
        };
        for handle in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            // SAFETY: this process's own handle; one that is missing is refused.
            unsafe { SetHandleInformation(GetStdHandle(handle), HANDLE_FLAG_INHERIT, 0) };
        }
    }

    /// `None` if not installed. The server started at login has no service to stop it: it runs until logout.
    #[expect(unsafe_code, reason = "deletes a registry value named by NUL-terminated strings")]
    pub(super) fn uninstall() -> Result<Option<Logged>> {
        if sys::is_root() {
            return uninstall_service();
        }
        _ = std::fs::remove_file(start_menu()?);
        // SAFETY: NUL-terminated strings.
        match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(RUN).as_ptr(), wide(NAME).as_ptr()) } {
            ERROR_SUCCESS => Ok(Some(Logged::UninstalledRunning { file: run_key() })),
            ERROR_FILE_NOT_FOUND => Ok(None),
            status => Err(os_error(status).into()),
        }
    }
}

#[cfg(not(any(target_os = "linux", target_vendor = "apple", windows)))]
mod imp {
    use super::{Error, PathBuf, Result};

    pub(super) fn install(_listen: &str, _exe: &std::path::Path, _serve: &[String]) -> Result<PathBuf> {
        Err(Error::Unsupported.into())
    }

    pub(super) fn uninstall() -> Result<Option<super::Logged>> {
        Err(Error::Unsupported.into())
    }
}

/// The arguments of the installed server: its settings are in the env file, so that a server started
/// another way runs the same.
pub(crate) fn serve_args() -> Vec<String> {
    vec!["serve".into()]
}

/// Installing again updates the executable and `PATH`, and restarts the server.
pub(crate) fn install(listen: &str) -> Result<Logged> {
    let exe = place::target()?;
    // Windows keeps a running executable from being replaced. Stopped rather than renamed aside, which
    // Windows allows: Defender quarantined an executable renamed aside while it ran and replaced, with the
    // Run key and the Start menu entry, as a trojan (Trojan:Win32/Bearfoos.A!ml).
    #[cfg(windows)]
    imp::stop_running(&exe)?;
    place::place(&exe)?;
    let file = imp::install(listen, &exe, &serve_args())?.display().to_string();
    Ok(if with_computer() { Logged::Installed { file } } else { Logged::InstalledAtLogin { file } })
}

/// Root's server, and on Linux a headless user's, which `install` keeps running without a login.
fn with_computer() -> bool {
    #[cfg(target_os = "linux")]
    return crate::sys::is_root() || imp::headless();
    #[cfg(not(target_os = "linux"))]
    crate::sys::is_root()
}

pub(crate) fn uninstall() -> Result<Logged> {
    Ok(imp::uninstall()?.unwrap_or(Logged::NotInstalled))
}

/// Whether the installed server starts at login; `None` if it is not installed.
#[cfg(tray)]
pub(crate) fn at_login() -> Option<bool> {
    imp::at_login()
}

/// Switches the installed server's start at login, and leaves the running one alone.
#[cfg(tray)]
pub(crate) fn set_at_login(on: bool) -> Result<()> {
    imp::set_at_login(on)
}

/// Deletes the state directory: the runs and the login token. Never the profiles, nor
/// the repositories.
pub(crate) fn delete() -> Result<Option<Logged>> {
    let dir = crate::settings::state_dir()?;
    if !dir.exists() {
        return Ok(None);
    }
    std::fs::remove_dir_all(&dir)?;
    Ok(Some(Logged::Deleted { dir: dir.display().to_string() }))
}

#[cfg(test)]
mod tests {
    /// The files that Linux packages install: the desktop entry and the unit.
    #[test]
    fn generated_linux_packages() {
        crate::tests::generated(
            file!(),
            &[
                (format!("packaging/linux/{}.desktop", crate::NAME), &|| super::desktop_entry(crate::NAME)),
                (format!("packaging/linux/{}.service", crate::NAME), &super::unit),
                ("packaging/linux/PKGBUILD".into(), &pkgbuild),
            ],
        );
        assert!(super::desktop_entry(r#"/a b/c"d%"#).contains(r#"Exec="/a b/c\\"d%%" serve --open"#));
    }

    /// The Chocolatey package of the Windows archive that dist builds.
    #[test]
    fn generated_windows_package() {
        crate::tests::generated(
            file!(),
            &[
                (format!("packaging/chocolatey/{}.nuspec", crate::NAME), &nuspec),
                ("packaging/chocolatey/tools/chocolateyinstall.ps1".into(), &chocolatey_install),
                ("packaging/chocolatey/tools/chocolateyBeforeModify.ps1".into(), &chocolatey_before_modify),
            ],
        );
    }

    /// The Chocolatey package's metadata, from Cargo.toml, with the rustic that the tests run as its dependency.
    fn nuspec() -> String {
        let url = env!("CARGO_PKG_REPOSITORY");
        format!(
            r#"<package xmlns="http://schemas.microsoft.com/packaging/2015/06/nuspec.xsd">
  <metadata>
    <id>{name}</id>
    <!-- Set by .github/workflows/chocolatey.yml at each release. -->
    <version>0</version>
    <title>{title}</title>
    <authors>AlphaJack</authors>
    <owners>AlphaJack</owners>
    <projectUrl>{url}</projectUrl>
    <packageSourceUrl>{url}/tree/main/packaging/chocolatey</packageSourceUrl>
    <docsUrl>{url}#readme</docsUrl>
    <bugTrackerUrl>{url}/issues</bugTrackerUrl>
    <licenseUrl>{url}/blob/main/LICENSE</licenseUrl>
    <requireLicenseAcceptance>false</requireLicenseAcceptance>
    <copyright>AlphaJack</copyright>
    <iconUrl>https://cdn.jsdelivr.net/gh/{repository}@main/assets/images/icon.png</iconUrl>
    <tags>{tags}</tags>
    <summary>{description}</summary>
    <description>{description}</description>
    <releaseNotes>{url}/blob/main/CHANGELOG.md</releaseNotes>
    <dependencies>
      <dependency id="rustic" version="{rustic}" />
    </dependencies>
  </metadata>
  <files>
    <file src="tools\**" target="tools" />
  </files>
</package>
"#,
            name = crate::NAME,
            title = crate::APP_NAME,
            repository = url.trim_start_matches("https://github.com/"),
            tags = env!("KEYWORDS").replace(';', " "),
            description = env!("CARGO_PKG_DESCRIPTION"),
            rustic = *crate::rustic::TESTED,
        )
    }

    /// The Chocolatey package's install script, which unpacks the 64-bit Windows archive.
    fn chocolatey_install() -> String {
        let targets = crate::tests::dist_targets("windows");
        let target = targets.iter().find(|t| t.starts_with("x86_64")).map_or("", String::as_str);
        format!(
            r"$ErrorActionPreference = 'Stop'

# url64bit and checksum64: set by .github/workflows/chocolatey.yml at each release.
$packageArgs = @{{
  packageName    = $env:ChocolateyPackageName
  unzipLocation  = Split-Path -Parent $MyInvocation.MyCommand.Definition
  url64bit       = '{url}/releases/download/v0/{name}-{target}.zip'
  checksum64     = 'XXXXXX'
  checksumType64 = 'sha256'
}}

Install-ChocolateyZipPackage @packageArgs
",
            url = env!("CARGO_PKG_REPOSITORY"),
            name = crate::NAME,
        )
    }

    /// The script that Chocolatey runs before an upgrade or uninstall: it stops the rusticlone that runs from the
    /// package, as Windows keeps a running executable from being replaced. The `Run` key starts the new one at
    /// the next login.
    fn chocolatey_before_modify() -> String {
        format!(
            r#"$ErrorActionPreference = 'Stop'

# Windows keeps a running executable from being replaced: rusticlone started from this package is stopped.
$tools = Split-Path -Parent $MyInvocation.MyCommand.Definition
Get-CimInstance Win32_Process -Filter "Name = '{name}.exe'" |
  Where-Object {{ $_.ExecutablePath -like "$tools\*" }} |
  ForEach-Object {{
    Stop-Process -Id $_.ProcessId -Force
    Write-Host "Stopped {name} ($($_.ProcessId)) to replace it: launch it again from the Start menu"
  }}
"#,
            name = crate::NAME,
        )
    }

    /// The Arch package of the Linux archives that dist builds, from Cargo.toml.
    fn pkgbuild() -> String {
        let targets = crate::tests::dist_targets("linux");
        // Each Linux target, under Arch's name for its architecture.
        let arches: Vec<(&str, &str)> =
            (targets.iter().map(String::as_str)).map(|t| (t.split('-').next().unwrap_or(t), t)).collect();
        let each = |line: &dyn Fn(&str, &str) -> String| arches.iter().map(|(a, t)| line(a, t)).collect::<String>();
        format!(
            r#"
# Maintainer: AlphaJack <19962243+AlphaJack at users dot noreply dot github dot com>

_name={name}
pkgname=$_name-bin
# Set by .github/workflows/aur.yml at each release.
pkgver=0
pkgrel=1
pkgdesc="{description}"
arch=({arch})
url="{url}"
license=('{license}')
depends=('rustic')
provides=("$_name")
conflicts=("$_name")
options=('!strip' '!debug')
backup=("etc/$_name/$_name.env")
{sources}{sums}

_completion(){{
 install -d "$pkgdir/usr/share/${{2%/*}}"
 COMPLETE="$1" "$pkgdir/usr/bin/$_name" | sed "s|$pkgdir||g" > "$pkgdir/usr/share/$2"
}}

package(){{
 local target
 case "$CARCH" in
{cases} esac
 cd "$srcdir/$_name-$target"
 install -D -m 755 $_name "$pkgdir/usr/bin/$_name"
 install -D -m 644 $_name.service "$pkgdir/usr/lib/systemd/system/$_name.service"
 install -D -m 644 $_name.service "$pkgdir/usr/lib/systemd/user/$_name.service"
 install -D -m 600 $_name.env "$pkgdir/etc/$_name/$_name.env"
 install -D -m 644 $_name.desktop "$pkgdir/usr/share/applications/$_name.desktop"
 install -D -m 644 logo.svg "$pkgdir/usr/share/icons/hicolor/scalable/apps/$_name.svg"
 _completion bash "bash-completion/completions/$_name"
 _completion zsh "zsh/site-functions/_$_name"
 _completion fish "fish/vendor_completions.d/$_name.fish"
}}
"#,
            name = crate::NAME,
            description = env!("CARGO_PKG_DESCRIPTION"),
            url = env!("CARGO_PKG_REPOSITORY"),
            license = env!("CARGO_PKG_LICENSE"),
            arch = arches.iter().map(|(a, _)| format!("'{a}'")).collect::<Vec<_>>().join(" "),
            sources = each(&|a, t| format!(
                "source_{a}=(\"$pkgname-$pkgver-{t}.tar.gz::$url/releases/download/v$pkgver/$_name-{t}.tar.gz\")\n"
            )),
            sums = each(&|a, _| format!("b2sums_{a}=('XXXXXX')\n")),
            cases = each(&|a, t| format!("  \"{a}\") target=\"{t}\" ;;\n")),
        )
    }
}

#[cfg(all(test, target_vendor = "apple"))]
mod apple_tests {
    #[test]
    fn the_keywords_fit_a_short_property_list() {
        let keywords: Vec<&str> = env!("KEYWORDS").split(';').collect();
        let plist = super::imp::bplist(&keywords).expect("Cargo.toml's keywords are few, short and ASCII");
        assert!(
            plist.starts_with(b"bplist00")
                && keywords.iter().all(|k| plist.windows(k.len()).any(|w| w == k.as_bytes()))
        );
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
        let exe = super::imp::bundle(&app, std::path::Path::new("/usr/bin/true"), "1.0.0").unwrap();
        assert!(signed());
        std::fs::copy("/usr/bin/false", &exe).unwrap();
        assert!(!signed());
        super::imp::bundle(&app, &exe, "1.0.1").unwrap();
        let info = std::fs::read_to_string(app.join("Contents").join("Info.plist")).unwrap();
        assert!(signed() && info.contains("<string>1.0.1</string>"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
