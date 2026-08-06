use serde::{Deserialize, Serialize};

/// Portable RGBA color stored in graph documents.
///
/// Its transparent four-byte representation matches the historical `egui::Color32` serde shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GraphColor([u8; 4]);

impl GraphColor {
    /// Opaque red.
    pub const RED: Self = Self::from_rgb(255, 0, 0);
    /// Opaque blue.
    pub const BLUE: Self = Self::from_rgb(0, 0, 255);
    /// Opaque gray used by neutral graph elements.
    pub const GRAY: Self = Self::from_rgb(128, 128, 128);
    /// Opaque white.
    pub const WHITE: Self = Self::from_rgb(255, 255, 255);
    /// Fully transparent black.
    pub const TRANSPARENT: Self = Self::from_rgba_unmultiplied(0, 0, 0, 0);

    /// Creates an opaque RGB color.
    pub const fn from_rgb(red: u8, green: u8, blue: u8) -> Self {
        Self([red, green, blue, 255])
    }

    /// Creates a color from unmultiplied RGBA components.
    pub const fn from_rgba_unmultiplied(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self([red, green, blue, alpha])
    }

    /// Returns the red component.
    pub const fn red(self) -> u8 {
        self.0[0]
    }

    /// Returns the green component.
    pub const fn green(self) -> u8 {
        self.0[1]
    }

    /// Returns the blue component.
    pub const fn blue(self) -> u8 {
        self.0[2]
    }

    /// Returns the alpha component.
    pub const fn alpha(self) -> u8 {
        self.0[3]
    }

    /// Returns the unmultiplied RGBA components.
    pub const fn to_array(self) -> [u8; 4] {
        self.0
    }
}

impl Default for GraphColor {
    fn default() -> Self {
        Self::TRANSPARENT
    }
}

/// Portable two-dimensional position stored in graph documents.
///
/// Its `x`/`y` object representation matches the historical `egui::Pos2` serde shape.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphPosition {
    /// Horizontal graph-canvas coordinate.
    pub x: f32,
    /// Vertical graph-canvas coordinate.
    pub y: f32,
}

impl GraphPosition {
    /// The graph origin.
    pub const ZERO: Self = Self::new(0.0, 0.0);

    /// Creates a graph position from its coordinates.
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Translates the position by a portable coordinate delta.
    pub fn translate(&mut self, delta_x: f32, delta_y: f32) {
        self.x += delta_x;
        self.y += delta_y;
    }
}
