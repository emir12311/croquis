//! SVG-to-texture proof for Spike A.
//!
//! The final app may use Wokwi's SVG assets or a hand-drawn catalogue; this
//! module deliberately proves only the mechanics: parse SVG, rasterise it at a
//! deterministic size, and hand the pixels to egui.

use eframe::egui;

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
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    let size = tree.size();
    let scale = (width as f32 / size.width()).min(height as f32 / size.height());
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
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
}
