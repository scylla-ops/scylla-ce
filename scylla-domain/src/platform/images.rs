//! The images that a person uploads, the custom icon of a resource (a glyph of the icon set of the
//! web app with a color, or an image), the avatar of a person, and the glyph avatars that the
//! server draws.

mod glyph_avatar;

pub use glyph_avatar::{
    glyph_angle, GlyphAvatar, GlyphColor, GlyphFill, GlyphFlip, GlyphPaint, GlyphShape,
};

use crate::{DomainError, Id};

/// The id of an uploaded image.
pub type ImageId = Id<Image>;

/// An uploaded image. The server keeps only a square PNG of `Image::SIDE` pixels that it made
/// from the upload, so it never gives back a file of a person as it is. The only other image is
/// the SVG of a glyph avatar, which the server draws itself.
#[derive(Clone, Copy, Debug)]
pub struct Image;

impl Image {
    /// The maximum size of an upload, in bytes.
    pub const MAX_UPLOAD: usize = 1_048_576;
    /// The maximum width and height of an upload, in pixels.
    pub const MAX_SOURCE_SIDE: u32 = 4096;
    /// The width and the height of the image that the server keeps, in pixels.
    pub const SIDE: u32 = 256;
    /// The content type of the image that the server keeps.
    pub const CONTENT_TYPE: &'static str = "image/png";
    /// The content type of the SVG of a glyph avatar.
    pub const SVG_CONTENT_TYPE: &'static str = "image/svg+xml";
}

/// The bytes of an upload: 1 to `Image::MAX_UPLOAD` bytes. The server reads the format later.
#[derive(Clone, PartialEq, Eq)]
pub struct Upload(Vec<u8>);

impl Upload {
    /// Checks the size of the bytes.
    pub fn parse(bytes: Vec<u8>) -> Result<Self, DomainError> {
        if bytes.is_empty() {
            return Err(DomainError::Empty);
        }
        if bytes.len() > Image::MAX_UPLOAD {
            return Err(DomainError::TooLarge {
                max: Image::MAX_UPLOAD,
            });
        }
        Ok(Self(bytes))
    }

    /// Returns the bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl std::fmt::Debug for Upload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Upload({} bytes)", self.0.len())
    }
}

/// The name of a glyph of the icon set of the web app: 1 to `MAX_LEN` lower case letters,
/// digits or dashes, with a letter or a digit first. The server does not know the icon set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphName(String);

impl GlyphName {
    /// The maximum number of characters.
    pub const MAX_LEN: usize = 64;

    /// Checks the characters and the length of the name.
    pub fn parse(raw: String) -> Result<Self, DomainError> {
        let first = raw.chars().next().ok_or(DomainError::NotAGlyphName)?;
        let allowed = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
        if raw.len() > Self::MAX_LEN || first == '-' || !raw.chars().all(allowed) {
            return Err(DomainError::NotAGlyphName);
        }
        Ok(Self(raw))
    }

    /// Returns the name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The color of a glyph: `#rrggbb` in lower case, or the name of a color of the web app (1 to
/// `MAX_LEN` lower case letters, digits or dashes, with a letter first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconColor(String);

impl IconColor {
    /// The maximum number of characters of a color name.
    pub const MAX_LEN: usize = 32;

    /// Checks the color, and writes a hex color in lower case. An empty text is no color.
    pub fn parse(raw: String) -> Result<Option<Self>, DomainError> {
        if raw.is_empty() {
            return Ok(None);
        }
        if let Some(hex) = raw.strip_prefix('#') {
            if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Ok(Some(Self(raw.to_ascii_lowercase())));
            }
            return Err(DomainError::NotAColor);
        }
        let allowed = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
        let starts_with_letter = raw.starts_with(|c: char| c.is_ascii_lowercase());
        if raw.len() > Self::MAX_LEN || !starts_with_letter || !raw.chars().all(allowed) {
            return Err(DomainError::NotAColor);
        }
        Ok(Some(Self(raw)))
    }

    /// Returns the color as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A glyph of the icon set with a color. No color means the default color.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconGlyph {
    /// The name of the glyph.
    pub name: GlyphName,
    /// The color, or `None` for the default color.
    pub color: Option<IconColor>,
}

/// The custom icon of a resource. A resource with no custom icon shows its default icon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Icon {
    /// A glyph with a color.
    Glyph(IconGlyph),
    /// An uploaded image.
    Image(ImageId),
    /// The SVG of a glyph avatar, with its options.
    GlyphAvatar(ImageId, GlyphAvatar),
}

impl Icon {
    /// Makes the icon of the three columns of a row: the image first, else the glyph. Gives
    /// `None` for no custom icon, or for a glyph name that is not valid.
    pub fn of_columns(
        name: Option<String>,
        color: Option<String>,
        image: Option<ImageId>,
    ) -> Option<Self> {
        if let Some(image) = image {
            return Some(Self::Image(image));
        }
        let name = GlyphName::parse(name?).ok()?;
        let color = color.and_then(|color| IconColor::parse(color).ok().flatten());
        Some(Self::Glyph(IconGlyph { name, color }))
    }

    /// Adds the options of a glyph avatar to an image: the icon becomes the glyph avatar.
    pub fn with_glyph(self, glyph: Option<GlyphAvatar>) -> Self {
        match (self, glyph) {
            (Self::Image(image), Some(glyph)) => Self::GlyphAvatar(image, glyph),
            (icon, _) => icon,
        }
    }

    /// Returns the image of the icon, if it has one.
    pub fn image(&self) -> Option<ImageId> {
        match self {
            Self::Glyph(_) => None,
            Self::Image(image) | Self::GlyphAvatar(image, _) => Some(*image),
        }
    }
}

/// The avatar of a person: an uploaded image, and the color of its initials. A person with
/// neither shows the initials on the default color.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Avatar {
    /// The image, if the person has one.
    pub image: Option<ImageId>,
    /// The color of the initials, or `None` for the default color.
    pub color: Option<IconColor>,
    /// The options of the glyph avatar, when the image is one and the read gives them.
    pub glyph: Option<GlyphAvatar>,
}

impl Avatar {
    /// Makes the avatar of the two columns of a row. Gives `None` for no image and no valid color.
    pub fn of_columns(image: Option<ImageId>, color: Option<String>) -> Option<Self> {
        let color = color.and_then(|color| IconColor::parse(color).ok().flatten());
        (image.is_some() || color.is_some()).then_some(Self {
            image,
            color,
            glyph: None,
        })
    }

    /// Adds the options of the glyph avatar of the image, if it is one.
    pub fn with_glyph(mut self, glyph: Option<GlyphAvatar>) -> Self {
        if self.image.is_some() {
            self.glyph = glyph;
        }
        self
    }
}

/// A change of the custom icon of a resource.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IconChange {
    /// Remove the custom icon: the resource shows its default icon again.
    Default,
    /// Show a glyph with a color.
    Glyph(IconGlyph),
    /// Show a new image, which the server makes from these bytes.
    Image(Upload),
    /// Show a glyph avatar, which the server draws.
    GlyphAvatar(GlyphAvatar),
}

#[cfg(test)]
mod tests;
