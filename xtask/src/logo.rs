//! `cargo xtask logo`: renders `assets/images/logo.svg` into the images beside it that need pixels, so that
//! builds do not render them: the tray's opacity, a PNG for notifications, the Windows and macOS icons, and the
//! web UI's icons for a phone's home screen, in the sizes and color of `assets/manifest.webmanifest`. It keeps the images that already match, so that it changes no bytes.

use std::{fs, path::Path};

use anyhow::{Context, Result};
use resvg::{
    tiny_skia::{Pixmap, Transform},
    usvg::{Options, Tree},
};

/// The tray icon's side in pixels, the same on every system: 18 points at the two pixels per point of a
/// Retina or high-density display, as the other icons of a menu bar or taskbar.
const TRAY: u32 = 36;
/// The side of the PNG, which the Windows icon also holds.
const PNG: u32 = 256;
/// The side of the macOS icon.
const MACOS: u32 = 512;
/// How much a channel may differ from the committed image, as renderers round differently on each CPU.
const TOLERANCE: u8 = 2;

/// The macOS app's icon, as Apple draws its own: the logo, glowing in its own color, on a rounded square of
/// the size of Apple's icon grid, white fading to light grey with a bright rim and a shadow below, so that
/// macOS shows it as it is instead of on a grey tile. The logo goes between the two halves.
const MACOS_ICON: [&str; 2] = [
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024"><defs><linearGradient id="plate" x1="0" y1="100" x2="0" y2="924" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#e6e6e6"/></linearGradient><linearGradient id="rim" x1="0" y1="100" x2="0" y2="924" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#ffffff" stop-opacity=".4"/></linearGradient><filter id="lift" x="-10%" y="-10%" width="120%" height="125%"><feDropShadow dy="12" stdDeviation="14" flood-opacity=".22"/></filter><filter id="glow" x="-20%" y="-20%" width="140%" height="150%"><feDropShadow dy="10" stdDeviation="14" flood-color="#f74c00" flood-opacity=".35"/></filter></defs><rect x="100" y="100" width="824" height="824" rx="200" fill="url(#plate)" filter="url(#lift)"/><rect x="104" y="104" width="816" height="816" rx="196" fill="none" stroke="url(#rim)" stroke-width="8"/><rect x="100" y="100" width="824" height="824" rx="200" fill="none" stroke="#000000" stroke-opacity=".08" stroke-width="2"/><g filter="url(#glow)"><svg x="182" y="182" width="660" height="660">"##,
    "</svg></g></svg>",
];

/// The web UI's icon: the logo on an opaque square of the manifest's background color, which it fills to the
/// safe zone, as Safari and Chrome want for an installed app, which they crop to their own shape. Flat, as the
/// screen an installed app opens on shows it on that color.
fn web_icon(background: &str) -> [String; 2] {
    [
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" fill="{background}"/><svg x="16" y="16" width="68" height="68">"#
        ),
        "</svg></svg>".into(),
    ]
}

pub(crate) fn render() -> Result<()> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/images");
    let logo = fs::read_to_string(dir.join("logo.svg"))?;
    let options = Options::default();
    let tree = Tree::from_data(logo.as_bytes(), &options)?;
    let png = fitted(&tree, PNG)?.encode_png()?;
    let framed = |[before, after]: [&str; 2]| Tree::from_data(format!("{before}{logo}{after}").as_bytes(), &options);
    let manifest: serde_json::Value = serde_json::from_str(&fs::read_to_string(dir.join("../manifest.webmanifest"))?)?;
    let [before, after] = web_icon(manifest["background_color"].as_str().context("no background_color")?);
    let web = framed([&before, &after])?;
    let mut files = vec![
        // Only its opacity: the tray draws it in black or white to suit the theme.
        ("logo.alpha", fitted(&tree, TRAY)?.pixels().iter().map(|p| p.alpha()).collect()),
        ("logo.ico", ico(&png)?),
        ("logo.icns", icns(&whole(&framed(MACOS_ICON)?, MACOS)?.encode_png()?)?),
        ("logo.png", png),
    ];
    for icon in manifest["icons"].as_array().context("no icons")? {
        let (name, sizes) = (icon["src"].as_str().context("no src")?, icon["sizes"].as_str().context("no sizes")?);
        let side = sizes.split_once('x').context("sizes")?.0.parse()?;
        files.push((name, whole(&web, side)?.encode_png()?));
    }
    for (name, rendered) in files {
        let path = dir.join(name);
        // An image that differs only by another machine's rounding is kept, so that a run changes no bytes.
        let (new, old) = (pixels(name, &rendered)?, fs::read(&path).ok().map(|old| pixels(name, &old)).transpose()?);
        if old.is_some_and(|old| alike(&new, &old)) {
            continue;
        }
        fs::write(&path, rendered)?;
    }
    Ok(())
}

/// The drawing, without the margins of its view box, centered in a square `side` pixels wide.
#[expect(clippy::cast_precision_loss, reason = "a few pixels")]
fn fitted(tree: &Tree, side: u32) -> Result<Pixmap> {
    let (drawn, side_f) = (tree.root().abs_stroke_bounding_box(), side as f32);
    let scale = side_f / drawn.width().max(drawn.height());
    let mut pixmap = Pixmap::new(side, side).context("empty logo")?;
    let transform = Transform::from_translate(-drawn.x(), -drawn.y())
        .post_scale(scale, scale)
        .post_translate((side_f - drawn.width() * scale) / 2.0, (side_f - drawn.height() * scale) / 2.0);
    resvg::render(tree, transform, &mut pixmap.as_mut());
    Ok(pixmap)
}

/// The drawing with the margins of its view box, in a square `side` pixels wide.
#[expect(clippy::cast_precision_loss, reason = "a few pixels")]
fn whole(tree: &Tree, side: u32) -> Result<Pixmap> {
    let scale = side as f32 / tree.size().width().max(tree.size().height());
    let mut pixmap = Pixmap::new(side, side).context("empty icon")?;
    resvg::render(tree, Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Ok(pixmap)
}

/// A macOS icon file holding one PNG: its header, then the PNG as its one element, each with a type and a
/// size that counts its own header.
fn icns(png: &[u8]) -> Result<Vec<u8>> {
    let element = u32::try_from(png.len() + 8)?;
    let mut icns = b"icns".to_vec();
    icns.extend((element + 8).to_be_bytes());
    icns.extend(b"ic09");
    icns.extend(element.to_be_bytes());
    icns.extend(png);
    Ok(icns)
}

/// An icon file holding one 256-pixel PNG: its header, its one directory entry, then the PNG.
fn ico(png: &[u8]) -> Result<Vec<u8>> {
    const HEADER: u32 = 6 + 16;
    let size = u32::try_from(png.len())?;
    // Reserved, type 1 (icon), one image; then 0 for a width and height of 256, no palette, reserved,
    // one plane, 32 bits per pixel, the PNG's size and offset.
    let mut ico = [0u16, 1, 1].iter().flat_map(|n| n.to_le_bytes()).collect::<Vec<u8>>();
    ico.extend([0, 0, 0, 0]);
    ico.extend([1u16, 32].iter().flat_map(|n| n.to_le_bytes()));
    ico.extend([size, HEADER].iter().flat_map(|n| n.to_le_bytes()));
    ico.extend(png);
    Ok(ico)
}

/// The pixels of a file written by [`render`]: the PNG that each icon file holds after its headers.
fn pixels(name: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    let png = match name {
        "logo.alpha" => return Ok(bytes.to_vec()),
        "logo.ico" => bytes.get(22..),
        "logo.icns" => bytes.get(16..),
        _ => Some(bytes),
    };
    Ok(Pixmap::decode_png(png.context("truncated icon")?)?.take())
}

fn alike(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= TOLERANCE)
}
