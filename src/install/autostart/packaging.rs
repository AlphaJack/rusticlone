//! The files that Linux packages install, and `install` writes on Linux: the systemd unit and the desktop entry.

/// The systemd unit of the server, which packages install too. In English, as packages carry it.
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
fn translated(id: &str) -> Vec<(String, String)> {
    #[cfg(feature = "server")]
    return crate::i18n::translations(id);
    #[cfg(not(feature = "server"))]
    Vec::new()
}

/// An application that opens the web UI logged in, by running `exe`, quoted as desktop entries require.
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
