//! Read-only visual snapshots. Games supply plain shapes and existing player text;
//! SDL renders them without calling back into gameplay or speech.
//!
//! The batched painter and glyph atlas were extracted from Freight Fate
//! (43a92022), by Joshua Tubbs. Bundled Overpass fonts retain their OFL notice.

#[cfg(feature = "visuals")]
pub mod draw;
#[cfg(feature = "visuals")]
mod renderer;
#[cfg(feature = "visuals")]
pub mod text;
#[cfg(feature = "visuals")]
pub use renderer::{save_bmp, Renderer};

/// Coordinates in the scene's 1000 by 600 drawing area. The renderer fits
/// this area beside the text panel without stretching it.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Color,
    },
    Circle {
        x: f32,
        y: f32,
        r: f32,
        color: Color,
    },
    Line {
        from: (f32, f32),
        to: (f32, f32),
        width: f32,
        color: Color,
    },
    Label {
        x: f32,
        y: f32,
        text: String,
        color: Color,
    },
}

pub type Color = [u8; 3];
pub const PAPER: Color = [244, 240, 224];
pub const GOLD: Color = [255, 206, 112];
pub const TEAL: Color = [100, 220, 210];
pub const RED: Color = [255, 139, 139];

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub enabled: bool,
    pub text_percent: u16,
    pub high_contrast: bool,
    pub lines: Vec<String>,
    pub caption: String,
    /// Ordered speech requests, retained for keyboard review even when speech queues.
    pub captions: Vec<String>,
    pub shapes: Vec<Shape>,
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            enabled: true,
            text_percent: 100,
            high_contrast: false,
            lines: Vec::new(),
            caption: String::new(),
            captions: Vec::new(),
            shapes: Vec::new(),
        }
    }
}

impl Scene {
    pub fn from_lines(lines: Vec<String>) -> Self {
        Self {
            lines,
            ..Self::default()
        }
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        self.shapes.push(Shape::Rect { x, y, w, h, color });
    }

    pub fn circle(&mut self, x: f32, y: f32, r: f32, color: Color) {
        self.shapes.push(Shape::Circle { x, y, r, color });
    }

    pub fn line(&mut self, from: (f32, f32), to: (f32, f32), width: f32, color: Color) {
        self.shapes.push(Shape::Line {
            from,
            to,
            width,
            color,
        });
    }

    pub fn label(&mut self, x: f32, y: f32, text: impl Into<String>, color: Color) {
        self.shapes.push(Shape::Label {
            x,
            y,
            text: text.into(),
            color,
        });
    }
}
