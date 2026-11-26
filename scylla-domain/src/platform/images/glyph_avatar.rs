//! The glyph avatar: a figure of the DiceBear style "Glyphs" with explicit options, which the
//! server draws as an SVG.

use serde::{Deserialize, Serialize};

use crate::DomainError;

/// The shape of the headpiece of a glyph avatar: `variant01` to `variant35`, the shapes of the
/// style, or `scylla`, the Scylla head.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GlyphShape(String);

impl GlyphShape {
    /// The name of the Scylla head: the four dots of the Scylla mark.
    pub const SCYLLA: &'static str = "scylla";
    /// The number of shapes of the style.
    pub const VARIANTS: u32 = 35;

    /// Checks that the text names a shape of the style or the Scylla head.
    pub fn parse(raw: String) -> Result<Self, DomainError> {
        let variant = raw
            .strip_prefix("variant")
            .filter(|digits| digits.len() == 2 && digits.chars().all(|c| c.is_ascii_digit()))
            .and_then(|digits| digits.parse::<u32>().ok())
            .is_some_and(|number| (1..=Self::VARIANTS).contains(&number));
        if variant || raw == Self::SCYLLA {
            Ok(Self(raw))
        } else {
            Err(DomainError::NotAGlyphShape)
        }
    }

    /// Returns the name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for GlyphShape {
    type Error = DomainError;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::parse(raw)
    }
}

impl From<GlyphShape> for String {
    fn from(shape: GlyphShape) -> Self {
        shape.0
    }
}

/// A color of a glyph avatar: `#rrggbb`, kept in lower case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GlyphColor(String);

impl GlyphColor {
    /// Checks the color, and writes it in lower case.
    pub fn parse(raw: String) -> Result<Self, DomainError> {
        let hex = raw.strip_prefix('#').ok_or(DomainError::NotAHexColor)?;
        if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(DomainError::NotAHexColor);
        }
        Ok(Self(raw.to_ascii_lowercase()))
    }

    /// Returns the color as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for GlyphColor {
    type Error = DomainError;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::parse(raw)
    }
}

impl From<GlyphColor> for String {
    fn from(color: GlyphColor) -> Self {
        color.0
    }
}

/// The fill of a part of a glyph avatar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GlyphFill {
    /// One color.
    #[default]
    Solid,
    /// A linear gradient, in the direction of the angle.
    Linear,
    /// A radial gradient, from the center.
    Radial,
}

impl GlyphFill {
    /// Returns the name of the fill in the options of DiceBear.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Solid => "solid",
            Self::Linear => "linear",
            Self::Radial => "radial",
        }
    }
}

/// The mirror of a glyph avatar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GlyphFlip {
    /// No mirror.
    #[default]
    None,
    /// A mirror from left to right.
    Horizontal,
    /// A mirror from top to bottom.
    Vertical,
    /// The two mirrors.
    Both,
}

impl GlyphFlip {
    /// Returns the name of the mirror in the options of DiceBear.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
            Self::Both => "both",
        }
    }
}

/// Checks an angle in degrees: -360 to 360.
pub fn glyph_angle(degrees: i32) -> Result<i32, DomainError> {
    if (-360..=360).contains(&degrees) {
        Ok(degrees)
    } else {
        Err(DomainError::NotAnAngle)
    }
}

/// The paint of a part of a glyph avatar: one color, or a gradient of 2 to 4 colors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlyphPaint {
    /// The colors, in their order.
    pub colors: Vec<GlyphColor>,
    /// The fill.
    pub fill: GlyphFill,
    /// The direction of a linear gradient, in degrees.
    pub angle: i32,
}

impl GlyphPaint {
    /// The most colors of a gradient.
    pub const MAX_STOPS: usize = 4;

    /// Checks the number of colors for the fill, and the angle.
    pub fn new(colors: Vec<GlyphColor>, fill: GlyphFill, angle: i32) -> Result<Self, DomainError> {
        let count = match fill {
            GlyphFill::Solid => colors.len() == 1,
            GlyphFill::Linear | GlyphFill::Radial => (2..=Self::MAX_STOPS).contains(&colors.len()),
        };
        if !count {
            return Err(DomainError::GlyphColorCount);
        }
        let angle = glyph_angle(angle)?;
        Ok(Self {
            colors,
            fill,
            angle,
        })
    }

    /// Returns the default background: solid white.
    pub fn white() -> Self {
        Self {
            colors: vec![GlyphColor("#ffffff".to_owned())],
            fill: GlyphFill::Solid,
            angle: 0,
        }
    }
}

/// A glyph avatar: the shape of its headpiece, its paints and its transform. The same options
/// always give the same SVG.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlyphAvatar {
    /// The shape of the headpiece.
    pub shape: GlyphShape,
    /// The paint of the headpiece and of the shoulders.
    pub glyph: GlyphPaint,
    /// The paint of the background.
    pub background: GlyphPaint,
    /// The mirror.
    pub flip: GlyphFlip,
    /// The rotation, in degrees.
    pub rotate: i32,
    /// The size, in percent.
    pub scale: u32,
}

impl GlyphAvatar {
    /// The smallest size, in percent.
    pub const MIN_SCALE: u32 = 50;
    /// The largest size, in percent.
    pub const MAX_SCALE: u32 = 150;
    /// The size of a figure with no size, in percent.
    pub const DEFAULT_SCALE: u32 = 100;

    /// Checks a size in percent. 0 gives the default size.
    pub fn scale(percent: i32) -> Result<u32, DomainError> {
        let range = Self::MIN_SCALE..=Self::MAX_SCALE;
        match u32::try_from(percent) {
            Ok(0) => Ok(Self::DEFAULT_SCALE),
            Ok(percent) if range.contains(&percent) => Ok(percent),
            _ => Err(DomainError::OutOfRange {
                min: Self::MIN_SCALE.into(),
                max: Self::MAX_SCALE.into(),
            }),
        }
    }

    /// Reads the options that a row keeps. Options that are not valid any more give `None`.
    pub fn of_json(value: serde_json::Value) -> Option<Self> {
        serde_json::from_value(value).ok()
    }

    /// Writes the options as the JSON that a row keeps.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
