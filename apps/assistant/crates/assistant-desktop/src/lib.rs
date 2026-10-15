//! Computer use: reading app windows (the accessibility tree, screenshots, OCR), numbering
//! what can be operated ("marks"), and acting on it (accessibility actions first, then the
//! pointer and keyboard), with each action checked afterwards.

pub mod a11y;
pub mod guard;
pub mod input;
pub mod marks;
pub mod ocr;
pub mod screen;
pub mod verify;

use serde::{Deserialize, Serialize};

/// A box in pixels: window-relative or screen (layout) coordinates, as the field says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    /// Shifted by a window's top-left corner: window-relative to screen coordinates.
    pub fn offset(self, dx: i64, dy: i64) -> Self {
        Self { x: self.x + dx as i32, y: self.y + dy as i32, ..self }
    }

    pub fn center(self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    pub fn is_empty(self) -> bool {
        self.w <= 0 || self.h <= 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_to_screen() {
        let r = Rect { x: 10, y: 20, w: 30, h: 40 }.offset(100, 200);
        assert_eq!(r, Rect { x: 110, y: 220, w: 30, h: 40 });
        assert_eq!(r.center(), (125, 240));
        assert!(Rect { x: 0, y: 0, w: 0, h: 5 }.is_empty());
    }
}
