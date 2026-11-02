//! The tray's icon: the logo in the bar's ink, with a dot in the color of what needs attention.

use crate::log::Level;

/// The logo's opacity, rendered by `cargo xtask logo`: the tray draws it in black or white to suit the theme.
pub(super) const LOGO: &[u8] = include_bytes!("../../../assets/images/logo.alpha");
/// The side of the square that `LOGO` fills, in pixels.
#[expect(clippy::cast_possible_truncation, reason = "a few dozen pixels")]
pub(super) const SIDE: u32 = LOGO.len().isqrt() as u32;
/// The dot's radius, and the gap cut around it, in pixels.
pub(super) const DOT: f32 = 5.0;
pub(super) const GAP: f32 = 2.0;
/// Red, green and blue.
pub(super) type Rgb = [u8; 3];
/// The dot's colors, as `--warning` and `--failure` in `style.css`.
pub(super) const WARNING: Rgb = [0xca, 0x8a, 0x04];
pub(super) const FAILURE: Rgb = [0xdc, 0x26, 0x26];
pub(super) const WHITE: Rgb = [u8::MAX; 3];
pub(super) const BLACK: Rgb = [0; 3];
/// From a pixel's corner to its center.
pub(super) const HALF_PIXEL: f32 = 0.5;

/// The logo in black on a light bar, in white on a dark one.
pub(super) fn ink(dark: bool) -> Rgb {
    if dark { WHITE } else { BLACK }
}

/// The dot's color, as the web UI shows it, unless all is well.
pub(super) fn dot(level: Level) -> Option<Rgb> {
    match level {
        Level::Success => None,
        Level::Warning => Some(WARNING),
        Level::Failure => Some(FAILURE),
    }
}

/// The logo's RGBA pixels in `ink`, cut around the level's dot in its top right corner, which is drawn unless `cut_only`,
/// as macOS draws it apart.
pub(super) fn pixels(level: Level, ink: Rgb, cut_only: bool) -> Vec<u8> {
    let ([red, green, blue], color) = (ink, dot(level));
    // Top right, as the badges of the Dock and of other menu bar icons.
    #[expect(clippy::cast_precision_loss, reason = "a few pixels")]
    let (cx, cy) = (SIDE as f32 - DOT, DOT);
    LOGO.iter()
        .zip(0..)
        .flat_map(|(&opacity, i)| {
            let Some([dot_red, dot_green, dot_blue]) = color else { return [red, green, blue, opacity] };
            #[expect(clippy::cast_precision_loss, reason = "a few pixels")]
            let (x, y) = ((i % SIDE) as f32 + HALF_PIXEL, (i / SIDE) as f32 + HALF_PIXEL);
            let distance = (x - cx).hypot(y - cy);
            // Antialiased: an edge pixel is as opaque as the shape covers it.
            let cover = |radius: f32| (radius + HALF_PIXEL - distance).clamp(0.0, 1.0);
            let (dot, logo) = (if cut_only { 0.0 } else { cover(DOT) }, 1.0 - cover(DOT + GAP));
            #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "within 0..=255")]
            let alpha = |a: f32| (a * f32::from(u8::MAX)).round() as u8;
            if dot > 0.0 {
                [dot_red, dot_green, dot_blue, alpha(dot)]
            } else {
                [red, green, blue, alpha(f32::from(opacity) / f32::from(u8::MAX) * logo)]
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pixels at the dot's center, in the top right corner.
    fn center(pixels: &[u8]) -> &[u8] {
        let (x, y) = (SIDE - 5, 5);
        let i = 4 * (y * SIDE + x) as usize;
        &pixels[i..i + 4]
    }

    #[test]
    fn a_dot_shows_what_needs_attention() {
        let well = pixels(Level::Success, ink(true), false);
        assert_eq!(well.len(), 4 * LOGO.len());
        assert!(well.chunks(4).all(|p| p[..3] == WHITE), "only the logo, in its ink");
        let failing = pixels(Level::Failure, ink(false), false);
        assert_eq!(center(&failing), [FAILURE[0], FAILURE[1], FAILURE[2], u8::MAX]);
        assert_eq!(center(&pixels(Level::Warning, ink(false), false))[..3], WARNING);
        // macOS draws the dot apart: only the gap around it is cut from the logo.
        assert_eq!(center(&pixels(Level::Failure, ink(false), true))[3], 0);
        assert_eq!(dot(Level::Success), None);
    }
}
