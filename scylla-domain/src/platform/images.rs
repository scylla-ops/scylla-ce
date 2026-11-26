//! The images that a person uploads, and the custom icon of a resource: a glyph of the icon set
//! of the web app with a color, or an image.

use crate::{DomainError, Id};

/// The id of an uploaded image.
pub type ImageId = Id<Image>;

/// An uploaded image. The server keeps only a square PNG of `Image::SIDE` pixels that it made
/// from the upload, so it never gives back a file of a person as it is.
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
}

#[cfg(test)]
mod tests {
    //! Unit tests of the values of an icon.

    use super::{GlyphName, Icon, IconColor, Image, ImageId, Upload};
    use crate::DomainError;

    /// A glyph name has lower case letters, digits and dashes, with no dash first.
    #[test]
    fn glyph_name_takes_lower_case_letters_digits_and_dashes() {
        // given
        let long = "a".repeat(GlyphName::MAX_LEN + 1);

        // when
        let names = [
            "rocket",
            "git-branch",
            "3d",
            "",
            "-x",
            "Rocket",
            "a b",
            &long,
        ]
        .map(|name| GlyphName::parse(name.to_owned()).is_ok());

        // then
        assert_eq!(names, [true, true, true, false, false, false, false, false]);
    }

    /// A color is a hex color, kept in lower case, or a color name. Empty is no color.
    #[test]
    fn icon_color_takes_a_hex_color_or_a_name() {
        // given
        let parse = |text: &str| IconColor::parse(text.to_owned());

        // when
        let hex = parse("#A1b2C3").unwrap().unwrap();
        let name = parse("blue-500").unwrap().unwrap();

        // then
        assert_eq!(hex.as_str(), "#a1b2c3");
        assert_eq!(name.as_str(), "blue-500");
        assert_eq!(parse(""), Ok(None));
        for bad in ["#abc", "#gggggg", "1blue", "Blue", "bl ue", "#a1b2c3d"] {
            assert_eq!(parse(bad), Err(DomainError::NotAColor), "{bad}");
        }
    }

    /// An upload has 1 to `Image::MAX_UPLOAD` bytes.
    #[test]
    fn upload_has_a_size_limit() {
        // given
        let max = vec![0; Image::MAX_UPLOAD];
        let over = vec![0; Image::MAX_UPLOAD + 1];

        // when
        let results = [
            Upload::parse(Vec::new()),
            Upload::parse(max),
            Upload::parse(over),
        ];

        // then
        assert_eq!(results[0], Err(DomainError::Empty));
        assert!(results[1].is_ok());
        assert_eq!(
            results[2],
            Err(DomainError::TooLarge {
                max: Image::MAX_UPLOAD
            })
        );
    }

    /// The image of a row wins over its glyph, and a row with no glyph name has no icon.
    #[test]
    fn icon_of_columns_takes_the_image_first() {
        // given
        let image = ImageId::generate();
        let name = || Some("rocket".to_owned());

        // when
        let with_image = Icon::of_columns(name(), None, Some(image));
        let glyph = Icon::of_columns(name(), Some("#00FF00".to_owned()), None);
        let none = Icon::of_columns(None, Some("blue".to_owned()), None);

        // then
        assert_eq!(with_image, Some(Icon::Image(image)));
        let Some(Icon::Glyph(glyph)) = glyph else {
            panic!("a glyph");
        };
        assert_eq!(glyph.name.as_str(), "rocket");
        assert_eq!(
            glyph.color.map(|color| color.as_str().to_owned()),
            Some("#00ff00".to_owned())
        );
        assert_eq!(none, None);
    }
}
