//! `cargo xtask vendor [VERSION]`: downloads htmx and its SSE extension from the npm registry,
//! verifies the published checksum, writes them to `assets`, and records the version and checksums in
//! `[workspace.metadata.htmx]` of `Cargo.toml`, which the server's tests compare the files with.
//! `cargo xtask vendor --check`: verifies that the files match the locked version.
//! `cargo xtask fixtures`: see [`fixtures`].
//! `cargo xtask acyclic [--mermaid]`: see [`acyclic`].
//! `cargo xtask logo`: see [`logo`].
//! `cargo xtask translations`: see [`translations`].
//! `cargo xtask generate`: writes anew every generated file: the logo's renderings, and the files of the
//! `generated_*` tests, such as the env file, the book's reference pages and the Linux packages.
//! `cargo xtask screenshots [--setup | --only NAME... | --teardown]`: takes the book's screenshots anew, in Docker:
//! see `screenshots/scripts/run.sh`.

#![expect(clippy::disallowed_methods, reason = "developer tool using files, processes and the network")]
#![expect(clippy::disallowed_macros, reason = "developer tool: errors are printed, not handled")]

mod acyclic;
mod fixtures;
mod logo;
mod translations;

use std::{env, fs, io::Read, path::Path};

use anyhow::{Context, Result, bail};
use base64::Engine;
use mercurio::transport::{BlockingTransport, Request, Transport};
use ring::digest::{SHA256, SHA512, digest};

const PACKAGE: &str = "htmx.org";
/// The path in the package, and the vendored file name.
const FILES: [(&str, &str); 3] = [
    ("package/dist/htmx.min.js", "htmx.min.js"),
    ("package/dist/ext/hx-sse.min.js", "hx-sse.min.js"),
    ("package/LICENSE", "htmx.LICENSE"),
];

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["vendor"] => vendor(None, false),
        ["vendor", "--check"] => vendor(None, true),
        ["vendor", version] => vendor(Some(version), false),
        ["fixtures"] => fixtures::record(),
        ["acyclic"] => acyclic::check(false),
        ["acyclic", "--mermaid"] => acyclic::check(true),
        ["logo"] => logo::render(),
        ["translations"] => translations::report(),
        ["generate"] => generate(),
        ["screenshots", ref options @ ..] => screenshots(options),
        _ => bail!(
            "usage: cargo xtask vendor [VERSION | --check] | cargo xtask fixtures | cargo xtask acyclic [--mermaid] | cargo xtask logo | cargo xtask translations | cargo xtask generate | cargo xtask screenshots [--setup | --only NAME... | --teardown]"
        ),
    }
}

fn generate() -> Result<()> {
    logo::render()?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let tests = || {
        let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let args = ["test", "--quiet", "--lib", "::generated_"];
        std::process::Command::new(cargo).args(args).current_dir(&root).status()
    };
    // A test fails when it writes its file, so that a stale one fails the checks: the second run passes.
    if !tests()?.success() && !tests()?.success() {
        bail!("the generated_* tests fail without writing their files");
    }
    Ok(())
}

fn screenshots(options: &[&str]) -> Result<()> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../screenshots/scripts/run.sh");
    if !std::process::Command::new(script).args(options).status()?.success() {
        bail!("the screenshots were not taken");
    }
    Ok(())
}

fn vendor(version: Option<&str>, check: bool) -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let assets = root.join("assets");
    let manifest_path = root.join("Cargo.toml");
    let mut manifest: toml_edit::DocumentMut = fs::read_to_string(&manifest_path)?.parse()?;
    let version = match version {
        Some(v) => v.to_owned(),
        None => manifest["workspace"]["metadata"]["htmx"]["version"]
            .as_str()
            .context("no version locked yet: cargo xtask vendor VERSION")?
            .to_owned(),
    };

    let meta: serde_json::Value =
        serde_json::from_slice(&get(&format!("https://registry.npmjs.org/{PACKAGE}/{version}"))?)?;
    let tarball = meta["dist"]["tarball"].as_str().context("no tarball in the registry")?;
    let integrity = meta["dist"]["integrity"].as_str().context("no integrity in the registry")?;
    let bytes = get(tarball)?;
    let got = format!("sha512-{}", base64::engine::general_purpose::STANDARD.encode(digest(&SHA512, &bytes)));
    if got != integrity {
        bail!("{tarball} is not what the registry vouches for ({got}, expected {integrity})");
    }

    let mut files = Vec::new();
    for entry in tar::Archive::new(flate2::read::GzDecoder::new(&bytes[..])).entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().into_owned();
        if let Some((_, name)) = FILES.iter().find(|(from, _)| *from == path) {
            let mut content = Vec::new();
            entry.read_to_end(&mut content)?;
            files.push((*name, content));
        }
    }
    if files.len() != FILES.len() {
        bail!("{PACKAGE}@{version} lacks some of {FILES:?}");
    }

    if check {
        for (name, content) in &files {
            if fs::read(assets.join(name)).ok().as_ref() != Some(content) {
                bail!("{name} is not the one of {PACKAGE}@{version}: cargo xtask vendor");
            }
        }
        println!("{PACKAGE}@{version}: the vendored files match");
        return Ok(());
    }
    let mut sha256 = toml_edit::Table::new();
    for (name, content) in &files {
        fs::write(assets.join(name), content)?;
        sha256[*name] = toml_edit::value(sha256_hex(content));
    }
    let htmx = &mut manifest["workspace"]["metadata"]["htmx"];
    htmx["version"] = toml_edit::value(version.as_str());
    htmx["integrity"] = toml_edit::value(integrity);
    htmx["sha256"] = toml_edit::Item::Table(sha256);
    fs::write(&manifest_path, manifest.to_string())?;
    println!("{PACKAGE}@{version}: {} files into {}", files.len(), assets.display());
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    digest(&SHA256, bytes).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn get(url: &str) -> Result<Vec<u8>> {
    let response = BlockingTransport::new().http(&Request::get(url).max_body(64 << 20))?;
    if !response.is_success() {
        bail!("{url}: HTTP {}", response.status);
    }
    Ok(response.body)
}
