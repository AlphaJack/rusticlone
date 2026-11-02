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
    AutoUpdate {
        file: String,
    },
    /// `install --auto-update` of an executable that its package manager updates.
    #[cfg(feature = "self-update")]
    AutoUpdateManaged,
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
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;

#[cfg(target_vendor = "apple")]
mod macos;
#[cfg(target_vendor = "apple")]
use macos as imp;

/// From an elevated shell, a system service that starts with the computer, restarted after a failure; else
/// the user's `Run` key, so that the server starts in the user's session at login, reads the user's profiles,
/// and shows its tray icon and desktop notifications, and a Start menu entry that opens the web UI.
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

#[cfg(not(any(target_os = "linux", target_vendor = "apple", windows)))]
mod other;
#[cfg(not(any(target_os = "linux", target_vendor = "apple", windows)))]
use other as imp;

/// The arguments of the installed server: its settings are in the env file, so that a server started
/// another way runs the same.
pub(crate) fn serve_args() -> Vec<String> {
    vec!["serve".into()]
}

/// Puts this executable where the server runs it, and returns where. Installing again updates it and `PATH`.
pub(crate) fn place() -> Result<PathBuf> {
    let exe = place::target()?;
    // Windows keeps a running executable from being replaced. Stopped rather than renamed aside, which
    // Windows allows: Defender quarantined an executable renamed aside while it ran and replaced, with the
    // Run key and the Start menu entry, as a trojan (Trojan:Win32/Bearfoos.A!ml).
    #[cfg(windows)]
    imp::stop_running(&exe)?;
    place::place(&exe)?;
    Ok(exe)
}

/// Installs the server that runs `exe`, from [`place()`]. Installing again restarts it.
pub(crate) fn install(listen: &str, exe: &Path) -> Result<Logged> {
    let file = imp::install(listen, exe, &serve_args())?.display().to_string();
    Ok(if with_computer() { Logged::Installed { file } } else { Logged::InstalledAtLogin { file } })
}

/// Root's server, and on Linux a headless user's, which `install` keeps running without a login.
pub(crate) fn with_computer() -> bool {
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
                ("packaging/chocolatey/tools/chocolateyUninstall.ps1".into(), &chocolatey_uninstall),
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
            r#"$ErrorActionPreference = 'Stop'
$restart = Join-Path $env:TEMP '{name}-restart.txt'

# url64bit and checksum64: set by .github/workflows/chocolatey.yml at each release.
$packageArgs = @{{
  packageName    = $env:ChocolateyPackageName
  unzipLocation  = Split-Path -Parent $MyInvocation.MyCommand.Definition
  url64bit       = '{url}/releases/download/v0/{name}-{target}.zip'
  checksum64     = 'XXXXXX'
  checksumType64 = 'sha256'
}}

Install-ChocolateyZipPackage @packageArgs

# The {name} that chocolateyBeforeModify.ps1 stopped, started again for its users: in their session and without
# this one's elevation, as at their login. A launcher that exits starts it, as removing a running task stops it.
if (Test-Path $restart) {{
  $exe = Join-Path $packageArgs.unzipLocation '{name}.exe'
  foreach ($user in Get-Content $restart) {{
    $task = "{name}-restart-$([guid]::NewGuid())"
    $launch = "Start-Process -FilePath '$exe' -ArgumentList serve -WindowStyle Hidden"
    $action = New-ScheduledTaskAction -Execute 'conhost.exe' -Argument "--headless powershell.exe -NoProfile -Command `"$launch`""
    $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    Register-ScheduledTask -TaskName $task -Action $action -Principal $principal -Settings $settings | Out-Null
    Start-ScheduledTask -TaskName $task
    $deadline = (Get-Date).AddSeconds(30)
    while ((Get-ScheduledTask -TaskName $task).State -eq 'Running' -and (Get-Date) -lt $deadline) {{ Start-Sleep 1 }}
    Unregister-ScheduledTask -TaskName $task -Confirm:$false
    Write-Host "Started {name} again for $user"
  }}
  Remove-Item $restart
}}
"#,
            url = env!("CARGO_PKG_REPOSITORY"),
            name = crate::NAME,
        )
    }

    /// The script that Chocolatey runs before an upgrade or uninstall: it stops the rusticlone that runs from the
    /// package, as Windows keeps a running executable from being replaced, and notes its users, for whom the
    /// install script starts the new one.
    fn chocolatey_before_modify() -> String {
        format!(
            r#"$ErrorActionPreference = 'Stop'
$restart = Join-Path $env:TEMP '{name}-restart.txt'

# Windows keeps a running executable from being replaced: {name} started from this package is stopped.
$tools = Split-Path -Parent $MyInvocation.MyCommand.Definition
$users = Get-CimInstance Win32_Process -Filter "Name = '{name}.exe'" |
  Where-Object {{ $_.ExecutablePath -like "$tools\*" }} |
  ForEach-Object {{
    $owner = Invoke-CimMethod -InputObject $_ -MethodName GetOwner
    Stop-Process -Id $_.ProcessId -Force
    "$($owner.Domain)\$($owner.User)"
  }}
if ($users) {{
  $users | Sort-Object -Unique | Set-Content $restart
}}
"#,
            name = crate::NAME,
        )
    }

    /// The script that Chocolatey runs to uninstall: no install follows to start rusticlone again.
    fn chocolatey_uninstall() -> String {
        format!(
            r"$ErrorActionPreference = 'Stop'

# Noted by chocolateyBeforeModify.ps1 for an upgrade's install script, which does not follow an uninstall.
Remove-Item (Join-Path $env:TEMP '{name}-restart.txt') -ErrorAction SilentlyContinue
",
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
optdepends=('rclone: use rclone remotes for rustic repositories')
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

    /// The app copies Homebrew's executable and signs the copy, which changes it: the app is not stale then, else
    /// each start would copy it again and restart. It is once Homebrew upgrades its executable.
    #[test]
    fn the_app_copies_homebrews_executable_again_only_once_it_changed() {
        let dir = std::env::temp_dir().join(format!("{}-brew-app-{}", crate::NAME, std::process::id()));
        let (app, cellar) = (dir.join(format!("{}.app", crate::APP_NAME)), dir.join("Cellar/rusticlone/2.0.1/bin"));
        std::fs::create_dir_all(&cellar).unwrap();
        let source = cellar.join(crate::NAME);
        std::fs::copy("/usr/bin/true", &source).unwrap();
        let exe = super::imp::bundle(&app, &source, "2.0.1").unwrap();
        let signed_copy_differs = std::fs::read(&exe).unwrap() != std::fs::read(&source).unwrap();
        let after_install = super::imp::stale(&exe);
        std::fs::copy("/usr/bin/false", &source).unwrap();
        let after_upgrade = super::imp::stale(&exe);
        std::fs::remove_dir_all(dir).unwrap();
        assert!(signed_copy_differs, "codesign no longer changes the copy: the test proves less");
        assert_eq!(after_install, None);
        assert_eq!(after_upgrade, Some(source));
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
