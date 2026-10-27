//! Sets `cfg(tray)` for the systems with a tray icon, and gives the Windows executable its icon, name and
//! manifest. Rebuilds the catalogs on a new language. Hands the code the keywords of Cargo.toml, and where GitHub serves the logo.

/// From Windows 11 24H2 on, no console for the executable unless it starts in one, so that it starts at login
/// without a window, and still prints into the terminal that runs it.
#[cfg(feature = "desktop")]
const MANIFEST: &str = r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <application>
    <windowsSettings>
      <consoleAllocationPolicy xmlns="http://schemas.microsoft.com/SMI/2024/WindowsSettings">detached</consoleAllocationPolicy>
    </windowsSettings>
  </application>
</assembly>"#;

#[expect(clippy::disallowed_methods, reason = "a build script reads Cargo's environment and writes files")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo::rerun-if-changed=assets/images/logo.ico");
    println!("cargo::rerun-if-changed=Cargo.toml");
    // A new language's folder, which the catalogs embed.
    println!("cargo::rerun-if-changed=i18n");
    println!("cargo::rustc-check-cfg=cfg(tray)");
    // The updater picks the archive that dist built for this target.
    println!("cargo::rustc-env=TARGET={}", std::env::var("TARGET")?);
    // Cargo hands the code its package's description, not its keywords.
    let manifest: toml_edit::DocumentMut = std::fs::read_to_string("Cargo.toml")?.parse()?;
    let keywords = manifest["package"]["keywords"].as_array().ok_or("Cargo.toml has no keywords")?;
    let keywords: Vec<&str> = keywords.iter().filter_map(toml_edit::Value::as_str).collect();
    println!("cargo::rustc-env=KEYWORDS={}", keywords.join(";"));
    // [`crate::APP_NAME`], which the Windows resource also names.
    let name = std::env::var("CARGO_PKG_NAME")?;
    let app_name = name.get(..1).unwrap_or_default().to_uppercase() + name.get(1..).unwrap_or_default();
    println!("cargo::rustc-env=APP_NAME={app_name}");
    // The logo that `cargo xtask logo` renders, as GitHub serves the files of the default branch.
    let raw =
        std::env::var("CARGO_PKG_REPOSITORY")?.replace("https://github.com/", "https://raw.githubusercontent.com/");
    println!("cargo::rustc-env=LOGO_URL={raw}/main/assets/images/logo.png");
    let os = std::env::var("CARGO_CFG_TARGET_OS")?;
    if std::env::var_os("CARGO_FEATURE_DESKTOP").is_some() && matches!(os.as_str(), "macos" | "linux" | "windows") {
        println!("cargo::rustc-cfg=tray");
        #[cfg(feature = "desktop")]
        {
            if os == "windows" {
                // Also the icon and name that Explorer and Task Manager show, with the version from Cargo.toml.
                winresource::WindowsResource::new()
                    .set("ProductName", &app_name)
                    .set("FileDescription", &app_name)
                    .set_icon("assets/images/logo.ico")
                    .set_manifest(MANIFEST)
                    .compile()?;
            }
        }
    }
    Ok(())
}
