//! SVG-to-texture proof for Spike A.
//!
//! The final app may use Wokwi's SVG assets or a hand-drawn catalogue; this
//! module deliberately proves only the mechanics: parse SVG, rasterise it at a
//! deterministic size, and hand the pixels to egui.

use eframe::egui;

/// A small LED element embedded in the binary, so the rasteriser has something to
/// draw without a file on disk. Test-only: the canvas uses the extracted art.
#[cfg(test)]
pub const SPIKE_LED_SVG: &str = r##"
<svg xmlns="http://www.w3.org/2000/svg" width="96" height="128" viewBox="0 0 24 32">
  <path d="M8 18v11M16 18v11" stroke="#bfc7c9" stroke-width="1.4" fill="none"/>
  <circle cx="12" cy="12" r="9" fill="#d23a3a" stroke="#6d1818" stroke-width="1.2"/>
  <circle cx="12" cy="12" r="5.2" fill="#f05a5a" opacity="0.85"/>
  <circle cx="9.5" cy="9.5" r="1.6" fill="#ffd1d1" opacity="0.8"/>
  <path d="M12 21v4M12 25l-2-2M12 25l2-2" stroke="#6d1818" stroke-width="0.8" fill="none"/>
</svg>
"##;

/// Rasterise an SVG to a premultiplied RGBA image, fitted into the requested
/// box without changing its aspect ratio.
pub fn rasterize(svg: &str, width: u32, height: u32) -> Option<egui::ColorImage> {
    let mut options = resvg::usvg::Options::default();
    // The Wokwi board drawings contain labels and pin text. With an empty
    // font database resvg silently omits all of it. Pick families from the
    // fonts this machine actually has instead of naming a font that may not be
    // installed.
    let (sans, mono) = {
        let fonts = options.fontdb_mut();
        fonts.load_system_fonts();
        let mut sans = None;
        let mut mono = None;
        for face in fonts.faces() {
            let Some((name, _)) = face.families.first() else {
                continue;
            };
            if face.monospaced {
                mono.get_or_insert_with(|| name.clone());
            } else {
                sans.get_or_insert_with(|| name.clone());
            }
        }
        let sans = sans.unwrap_or_else(|| "sans-serif".to_string());
        let mono = mono.unwrap_or_else(|| "monospace".to_string());
        fonts.set_sans_serif_family(sans.clone());
        fonts.set_monospace_family(mono.clone());
        (sans, mono)
    };
    let _ = mono;
    options.font_family = sans;
    let tree = match resvg::usvg::Tree::from_str(svg, &options) {
        Ok(tree) => tree,
        Err(error) => {
            eprintln!("SVG parse failed: {error}");
            return None;
        }
    };
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    let size = tree.size();
    // The manifest box is the pin coordinate space: fill it exactly rather than
    // preserving the SVG aspect and letterboxing the drawing inside it.
    let scale_x = width as f32 / size.width();
    let scale_y = height as f32 / size.height();
    let transform = resvg::tiny_skia::Transform::from_scale(scale_x, scale_y);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [width as usize, height as usize],
        pixmap.data(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spike_led_rasterises_with_visible_pixels() {
        let image = rasterize(SPIKE_LED_SVG, 96, 128).expect("the embedded SVG should parse");
        assert_eq!(image.size, [96, 128]);
        assert!(
            image.pixels.iter().any(|pixel| pixel.a() > 0),
            "the raster should not be fully transparent"
        );
    }

    #[test]
    fn invalid_svg_is_rejected_instead_of_panicking() {
        assert!(rasterize("<svg><not-closed>", 32, 32).is_none());
    }

    #[test]
    fn system_fonts_render_svg_text() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="40">
            <rect width="120" height="40" fill="#000"/>
            <text x="8" y="26" font-size="22" font-family="sans-serif" fill="#fff">ABC</text>
        </svg>"##;
        let image = rasterize(svg, 120, 40).expect("text svg parses");
        assert!(
            image
                .pixels
                .iter()
                .any(|pixel| pixel.r() > 180 && pixel.g() > 180 && pixel.b() > 180),
            "rendered text should have light pixels"
        );
    }
}
