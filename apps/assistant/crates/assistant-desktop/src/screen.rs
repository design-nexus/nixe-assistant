//! Screenshots of a window or region with `grim`, and how image pixels map back to the
//! layout coordinates Hyprland and the pointer use (they differ on scaled monitors).

use crate::Rect;
use anyhow::{ensure, Context, Result};
use image::DynamicImage;

pub struct Shot {
    pub image: DynamicImage,
    /// What was captured, in layout coordinates.
    pub area: Rect,
}

impl Shot {
    /// Image pixels per layout unit (1.0 on an unscaled monitor).
    pub fn scale(&self) -> f64 {
        if self.area.w <= 0 {
            1.0
        } else {
            self.image.width() as f64 / self.area.w as f64
        }
    }

    /// An image-pixel box as a layout box.
    pub fn to_layout(&self, r: Rect) -> Rect {
        let s = self.scale();
        Rect {
            x: self.area.x + (r.x as f64 / s).round() as i32,
            y: self.area.y + (r.y as f64 / s).round() as i32,
            w: (r.w as f64 / s).round() as i32,
            h: (r.h as f64 / s).round() as i32,
        }
    }

    /// A layout box as an image-pixel box (clipped to the image).
    pub fn to_image(&self, r: Rect) -> Rect {
        let s = self.scale();
        let x = ((r.x - self.area.x) as f64 * s).round() as i32;
        let y = ((r.y - self.area.y) as f64 * s).round() as i32;
        let (iw, ih) = (self.image.width() as i32, self.image.height() as i32);
        let x0 = x.clamp(0, iw);
        let y0 = y.clamp(0, ih);
        let x1 = (x + (r.w as f64 * s).round() as i32).clamp(0, iw);
        let y1 = (y + (r.h as f64 * s).round() as i32).clamp(0, ih);
        Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }

    pub fn png(&self) -> Result<Vec<u8>> {
        let mut out = std::io::Cursor::new(Vec::new());
        self.image.write_to(&mut out, image::ImageFormat::Png)?;
        Ok(out.into_inner())
    }

    /// A copy no wider than `max_w`, for the vision model.
    pub fn downscaled(&self, max_w: u32) -> DynamicImage {
        if self.image.width() <= max_w {
            return self.image.clone();
        }
        let h = (self.image.height() as u64 * max_w as u64 / self.image.width() as u64) as u32;
        self.image.resize_exact(max_w, h.max(1), image::imageops::FilterType::Triangle)
    }
}

/// Capture an area of the screen (layout coordinates).
pub async fn capture(area: Rect) -> Result<Shot> {
    ensure!(!area.is_empty(), "nothing to capture");
    let geometry = format!("{},{} {}x{}", area.x, area.y, area.w, area.h);
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::process::Command::new("grim").args(["-g", &geometry, "-t", "png", "-"]).kill_on_drop(true).output(),
    )
    .await
    .context("grim timed out")?
    .context("grim isn't available")?;
    ensure!(out.status.success(), "grim failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    let image = image::load_from_memory_with_format(&out.stdout, image::ImageFormat::Png).context("grim's image")?;
    Ok(Shot { image, area })
}

/// Cells of a 192×192 grid that look different between two captures of the same area.
pub const GRID_CELLS: usize = 192 * 192;

/// The share of the region (0–1) that looks different between two captures of the same
/// area, on a coarse grayscale grid so cursor blink and antialiasing don't count.
pub fn changed_share(a: &DynamicImage, b: &DynamicImage) -> f64 {
    // Fine enough that a new tab label or a typed word shows; coarse enough to ignore noise.
    const GRID: u32 = 192;
    let small = |i: &DynamicImage| i.resize_exact(GRID, GRID, image::imageops::FilterType::Triangle).to_luma8();
    let (a, b) = (small(a), small(b));
    let differ = a.pixels().zip(b.pixels()).filter(|(p, q)| p.0[0].abs_diff(q.0[0]) > 16).count();
    differ as f64 / (GRID * GRID) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(w: u32, h: u32, area: Rect) -> Shot {
        Shot { image: DynamicImage::new_rgb8(w, h), area }
    }

    #[test]
    fn maps_between_image_and_layout() {
        // A 1.5× monitor: 300 layout units wide come out as 450 pixels.
        let s = shot(450, 300, Rect { x: 1000, y: 50, w: 300, h: 200 });
        assert!((s.scale() - 1.5).abs() < 1e-9);
        assert_eq!(s.to_layout(Rect { x: 150, y: 30, w: 45, h: 15 }), Rect { x: 1100, y: 70, w: 30, h: 10 });
        assert_eq!(s.to_image(Rect { x: 1100, y: 70, w: 30, h: 10 }), Rect { x: 150, y: 30, w: 45, h: 15 });
        // Clipped to the image.
        assert_eq!(s.to_image(Rect { x: 990, y: 40, w: 20, h: 20 }), Rect { x: 0, y: 0, w: 15, h: 15 });
    }

    #[test]
    fn change_share() {
        let a = DynamicImage::new_luma8(100, 100);
        let mut b = a.to_luma8();
        for y in 0..50 {
            for x in 0..100 {
                b.put_pixel(x, y, image::Luma([255]));
            }
        }
        let share = changed_share(&a, &DynamicImage::ImageLuma8(b));
        assert!((0.4..0.6).contains(&share), "{share}");
        assert_eq!(changed_share(&a, &a), 0.0);
    }
}
