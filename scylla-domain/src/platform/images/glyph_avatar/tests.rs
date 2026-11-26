//! Unit tests of the options of a glyph avatar.
use super::{GlyphAvatar, GlyphColor, GlyphFill, GlyphFlip, GlyphPaint, GlyphShape};
use crate::DomainError;

/// Gives the colors of the texts.
fn colors(texts: &[&str]) -> Vec<GlyphColor> {
    texts
        .iter()
        .map(|text| GlyphColor::parse((*text).to_owned()).unwrap())
        .collect()
}

/// A shape is one of the 35 variants of the style, or the Scylla head.
#[test]
fn glyph_shape_takes_the_variants_and_the_scylla_head() {
    // given
    let names = [
        "variant01",
        "variant35",
        "scylla",
        "variant00",
        "variant36",
        "variant1",
        "variant+1",
        "Scylla",
        "",
    ];

    // when
    let parsed = names.map(|name| GlyphShape::parse(name.to_owned()).is_ok());

    // then
    let expected = [true, true, true, false, false, false, false, false, false];
    assert_eq!(parsed, expected);
}

/// A color is `#rrggbb`, kept in lower case.
#[test]
fn glyph_color_takes_a_hex_color() {
    // given
    let parse = |text: &str| GlyphColor::parse(text.to_owned());

    // when
    let color = parse("#A1B2C3").unwrap();

    // then
    assert_eq!(color.as_str(), "#a1b2c3");
    for bad in ["a1b2c3", "#abc", "#gggggg", "blue", ""] {
        assert_eq!(parse(bad), Err(DomainError::NotAHexColor), "{bad}");
    }
}

/// A solid fill has one color, a gradient 2 to 4, and the angle is -360 to 360.
#[test]
fn glyph_paint_checks_the_colors_of_its_fill_and_its_angle() {
    // given
    let one = colors(&["#000000"]);
    let two = colors(&["#000000", "#ffffff"]);
    let five = colors(&["#000000"; 5]);

    // when
    let solid = GlyphPaint::new(one.clone(), GlyphFill::Solid, 0);
    let linear = GlyphPaint::new(two.clone(), GlyphFill::Linear, -360);
    let solid_two = GlyphPaint::new(two.clone(), GlyphFill::Solid, 0);
    let radial_one = GlyphPaint::new(one, GlyphFill::Radial, 0);
    let radial_five = GlyphPaint::new(five, GlyphFill::Radial, 0);
    let bad_angle = GlyphPaint::new(two, GlyphFill::Linear, 361);

    // then
    assert!(solid.is_ok() && linear.is_ok());
    assert_eq!(solid_two, Err(DomainError::GlyphColorCount));
    assert_eq!(radial_one, Err(DomainError::GlyphColorCount));
    assert_eq!(radial_five, Err(DomainError::GlyphColorCount));
    assert_eq!(bad_angle, Err(DomainError::NotAnAngle));
}

/// A size is 50 to 150 percent, and 0 gives 100.
#[test]
fn glyph_avatar_scale_has_a_range_and_a_default() {
    // given
    let out = Err(DomainError::OutOfRange { min: 50, max: 150 });

    // when
    let parsed = [0, 50, 150, 49, 151, -1].map(GlyphAvatar::scale);

    // then
    assert_eq!(parsed[..3], [Ok(100), Ok(50), Ok(150)]);
    assert_eq!(parsed[3..], [out.clone(), out.clone(), out]);
}

/// The JSON of a row gives the same options back, and a bad shape in it gives no options.
#[test]
fn glyph_avatar_json_round_trips() {
    // given
    let glyph = GlyphPaint::new(colors(&["#5d9972", "#205090"]), GlyphFill::Linear, 45);
    let avatar = GlyphAvatar {
        shape: GlyphShape::parse("scylla".to_owned()).unwrap(),
        glyph: glyph.unwrap(),
        background: GlyphPaint::white(),
        flip: GlyphFlip::Horizontal,
        rotate: -15,
        scale: 120,
    };
    let mut bad = avatar.to_json();
    bad["shape"] = "variant99".into();

    // when
    let back = GlyphAvatar::of_json(avatar.to_json());

    // then
    assert_eq!(back, Some(avatar));
    assert_eq!(GlyphAvatar::of_json(bad), None);
}
