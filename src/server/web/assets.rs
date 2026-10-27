//! Assets embedded in the binary, versioned by content, so that browsers cache them until an upgrade.

use std::{collections::BTreeMap, sync::LazyLock};

use axum::{extract::Path, http::header, response::IntoResponse};

use super::{Error, Missing, Result, Route};
use crate::sys;

const ASSETS: [(&str, &[u8], &str); 7] = [
    ("htmx.min.js", include_bytes!("../../../assets/htmx.min.js"), "text/javascript"),
    ("hx-sse.min.js", include_bytes!("../../../assets/hx-sse.min.js"), "text/javascript"),
    ("app.js", include_bytes!("../../../assets/app.js"), "text/javascript"),
    ("style.css", include_bytes!("../../../assets/style.css"), "text/css"),
    ("logo.svg", include_bytes!("../../../assets/images/logo.svg"), "image/svg+xml"),
    ("icon.png", include_bytes!("../../../assets/images/icon.png"), "image/png"),
    ("manifest.webmanifest", include_bytes!("../../../assets/manifest.webmanifest"), "application/manifest+json"),
];

pub(super) fn asset_url(name: &str) -> &'static str {
    static URLS: LazyLock<BTreeMap<&str, String>> = LazyLock::new(|| {
        ASSETS
            .iter()
            .map(|(name, body, _)| (*name, format!("{}?v={:x}", Route::Asset(name), sys::fnv1a(body))))
            .collect()
    });
    URLS.get(name).map_or("", String::as_str)
}

/// The page's background in the light and the dark scheme, from `--background` in the style sheet, for the
/// browser's bar around an installed app.
pub(super) fn theme_colors() -> [&'static str; 2] {
    static COLORS: LazyLock<[&str; 2]> = LazyLock::new(|| {
        include_str!("../../../assets/style.css")
            .split_once("--background: light-dark(")
            .and_then(|(_, rest)| rest.split_once(");"))
            .and_then(|(value, _)| value.split_once(", "))
            .map_or(["", ""], |(light, dark)| [light, dark])
    });
    *COLORS
}

pub(super) async fn asset(Path(file): Path<String>) -> Result<impl IntoResponse> {
    let (_, body, mime) = ASSETS.iter().find(|(n, ..)| *n == file).ok_or(Error::NotFound(Missing::Asset))?;
    Ok(([(header::CONTENT_TYPE, *mime), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")], *body))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vendored files match the checksums `cargo xtask vendor` recorded.
    #[test]
    fn vendored_files_are_the_locked_ones() {
        let locked: toml_edit::DocumentMut = include_str!("../../../Cargo.toml").parse().unwrap();
        let locked = &locked["workspace"]["metadata"]["htmx"]["sha256"];
        for name in ["htmx.min.js", "hx-sse.min.js"] {
            let (_, body, _) = ASSETS.iter().find(|(n, ..)| *n == name).unwrap();
            assert_eq!(sys::sha256(body), locked[name].as_str().unwrap(), "{name}: cargo xtask vendor");
        }
    }

    #[test]
    fn theme_colors_are_the_background() {
        assert_eq!(theme_colors(), ["hsl(30 8% 98%)", "hsl(220 5.5% 9%)"]);
    }

    /// Every icon of the manifest is served.
    #[test]
    fn manifest_icons_are_served() {
        let (_, manifest, _) = ASSETS.iter().find(|(n, ..)| *n == "manifest.webmanifest").unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(manifest).unwrap();
        for icon in manifest["icons"].as_array().unwrap() {
            let src = icon["src"].as_str().unwrap();
            assert!(ASSETS.iter().any(|(n, ..)| *n == src), "{src} is not in ASSETS");
        }
    }
}
