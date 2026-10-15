//! Numbered marks ("set of marks"): one list of the things on screen that can be pointed
//! at, from the accessibility tree and from OCR, each with a label and a screen box. The
//! planner can pick a mark by its label; the vision model sees the numbers drawn on the
//! screenshot and answers with a mark rather than guessing pixel coordinates.

use crate::a11y::Element;
use crate::ocr::Line;
use crate::screen::Shot;
use crate::Rect;
use image::{Rgba, RgbaImage};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Mark {
    pub id: String,
    pub label: String,
    /// "a11y" or "ocr".
    pub source: &'static str,
    /// The accessibility element behind it, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Layout (screen) coordinates.
    pub bounds: Rect,
}

fn overlap(a: Rect, b: Rect) -> f64 {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    if x1 <= x0 || y1 <= y0 {
        return 0.0;
    }
    let inter = ((x1 - x0) * (y1 - y0)) as f64;
    inter / ((a.w * a.h).min(b.w * b.h).max(1)) as f64
}

/// Build the marks: accessibility elements with a box (moved to screen coordinates by the
/// window's corner), then OCR lines that no element already covers.
pub fn build(elements: &[Element], window_at: (i64, i64), shot: Option<&Shot>, lines: &[Line]) -> Vec<Mark> {
    let mut marks = Vec::new();
    for e in elements {
        let Some(b) = e.bounds.filter(|b| !b.is_empty()) else { continue };
        let label = if e.name.is_empty() { e.text.clone().unwrap_or_default() } else { e.name.clone() };
        marks.push(Mark {
            id: String::new(),
            label,
            source: "a11y",
            element: Some(e.id.clone()),
            role: Some(e.role.clone()),
            bounds: b.offset(window_at.0, window_at.1),
        });
    }
    if let Some(shot) = shot {
        for l in lines {
            let b = shot.to_layout(l.bounds);
            if b.is_empty() || l.text.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
                continue;
            }
            if marks.iter().any(|m| m.source == "a11y" && overlap(m.bounds, b) > 0.6) {
                continue;
            }
            marks.push(Mark { id: String::new(), label: l.text.clone(), source: "ocr", element: None, role: None, bounds: b });
        }
    }
    for (i, m) in marks.iter_mut().enumerate() {
        m.id = format!("m{}", i + 1);
    }
    marks
}

/// 3×5 pixel digits and the letter m, for labels without a font dependency.
const GLYPHS: [(char, [u8; 5]); 11] = [
    ('0', [0b111, 0b101, 0b101, 0b101, 0b111]),
    ('1', [0b010, 0b110, 0b010, 0b010, 0b111]),
    ('2', [0b111, 0b001, 0b111, 0b100, 0b111]),
    ('3', [0b111, 0b001, 0b111, 0b001, 0b111]),
    ('4', [0b101, 0b101, 0b111, 0b001, 0b001]),
    ('5', [0b111, 0b100, 0b111, 0b001, 0b111]),
    ('6', [0b111, 0b100, 0b111, 0b101, 0b111]),
    ('7', [0b111, 0b001, 0b010, 0b010, 0b010]),
    ('8', [0b111, 0b101, 0b111, 0b101, 0b111]),
    ('9', [0b111, 0b101, 0b111, 0b001, 0b111]),
    ('m', [0b000, 0b000, 0b111, 0b111, 0b101]),
];

fn fill(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, c: Rgba<u8>) {
    for yy in y.max(0)..(y + h).min(img.height() as i32) {
        for xx in x.max(0)..(x + w).min(img.width() as i32) {
            img.put_pixel(xx as u32, yy as u32, c);
        }
    }
}

/// The screenshot with each mark's box outlined and its number on a tag.
pub fn draw(shot: &Shot, marks: &[Mark]) -> RgbaImage {
    let mut img = shot.image.to_rgba8();
    // Big enough to read after the image is scaled down for the model (to ~1280 wide).
    let px = (img.width() as i32 / 320).clamp(2, 12);
    let colors = [Rgba([230, 40, 40, 255]), Rgba([30, 120, 230, 255]), Rgba([20, 160, 60, 255]), Rgba([200, 90, 0, 255])];
    for (i, m) in marks.iter().enumerate() {
        let r = shot.to_image(m.bounds);
        if r.is_empty() {
            continue;
        }
        let c = colors[i % colors.len()];
        let t = (px / 2).max(1);
        fill(&mut img, r.x, r.y, r.w, t, c);
        fill(&mut img, r.x, r.y + r.h - t, r.w, t, c);
        fill(&mut img, r.x, r.y, t, r.h, c);
        fill(&mut img, r.x + r.w - t, r.y, t, r.h, c);
        let digits: Vec<char> = m.id.chars().skip(1).collect();
        let tag_w = (digits.len() as i32 * 4 + 1) * px;
        let tag_h = 7 * px;
        let (tx, ty) = (r.x, (r.y - tag_h).max(0));
        // A dark rim keeps the tag readable on any background.
        fill(&mut img, tx - 1, ty - 1, tag_w + 2, tag_h + 2, Rgba([0, 0, 0, 255]));
        fill(&mut img, tx, ty, tag_w, tag_h, c);
        for (k, ch) in digits.iter().enumerate() {
            let Some((_, rows)) = GLYPHS.iter().find(|(g, _)| g == ch) else { continue };
            for (row, bits) in rows.iter().enumerate() {
                for col in 0..3 {
                    if bits & (0b100 >> col) != 0 {
                        fill(&mut img, tx + px + (k as i32 * 4 + col) * px, ty + px + row as i32 * px, px, px, Rgba([255, 255, 255, 255]));
                    }
                }
            }
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::DynamicImage;

    fn element(id: &str, name: &str, b: Rect) -> Element {
        Element { id: id.into(), role: "push button".into(), name: name.into(), states: vec![], bounds: Some(b), actions: vec![], text: None }
    }

    #[test]
    fn merges_tree_and_ocr() {
        let shot = Shot { image: DynamicImage::new_rgb8(400, 200), area: Rect { x: 100, y: 50, w: 400, h: 200 } };
        let elements = [element("e1", "Save", Rect { x: 10, y: 10, w: 60, h: 20 }), element("e2", "", Rect { x: 0, y: 0, w: 0, h: 0 })];
        let lines = [
            // Covered by the Save button: dropped.
            Line { text: "Save".into(), bounds: Rect { x: 15, y: 12, w: 40, h: 14 }, confidence: 90.0 },
            Line { text: "Recent files".into(), bounds: Rect { x: 200, y: 100, w: 80, h: 14 }, confidence: 90.0 },
            Line { text: "|".into(), bounds: Rect { x: 300, y: 100, w: 4, h: 14 }, confidence: 90.0 },
        ];
        let marks = build(&elements, (100, 50), Some(&shot), &lines);
        assert_eq!(marks.len(), 2);
        assert_eq!((marks[0].id.as_str(), marks[0].label.as_str(), marks[0].source), ("m1", "Save", "a11y"));
        assert_eq!(marks[0].bounds, Rect { x: 110, y: 60, w: 60, h: 20 });
        assert_eq!((marks[1].id.as_str(), marks[1].label.as_str(), marks[1].source), ("m2", "Recent files", "ocr"));
        assert_eq!(marks[1].bounds, Rect { x: 300, y: 150, w: 80, h: 14 });
        let img = draw(&shot, &marks);
        // The outline of m1 is painted where its box is.
        assert_ne!(*img.get_pixel(10, 15), Rgba([0, 0, 0, 255]));
        assert_eq!(*img.get_pixel(150, 180), Rgba([0, 0, 0, 255]));
    }
}
