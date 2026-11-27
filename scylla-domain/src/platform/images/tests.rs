//! Unit tests of the values of an icon.
use super::{Avatar, GlyphName, Icon, IconColor, Image, ImageId, Upload};
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

/// A row with an image or a valid color has an avatar, and a bad color is no color.
#[test]
fn avatar_of_columns_needs_an_image_or_a_color() {
    // given
    let image = ImageId::generate();

    // when
    let with_image = Avatar::of_columns(Some(image), Some("Bad".to_owned()));
    let with_color = Avatar::of_columns(None, Some("chart-4".to_owned()));
    let none = Avatar::of_columns(None, Some("Bad".to_owned()));

    // then
    assert_eq!(with_image.map(|avatar| avatar.image), Some(Some(image)));
    let color = with_color.and_then(|avatar| avatar.color);
    assert_eq!(
        color.map(|color| color.as_str().to_owned()),
        Some("chart-4".to_owned())
    );
    assert_eq!(none, None);
}
