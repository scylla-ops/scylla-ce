//! Draws a glyph avatar as an SVG with `dicebear-core` and the style of the glyph avatars of
//! Scylla. The web app draws its preview with `@dicebear/core`, the same style file and the same
//! options, so the two SVGs are the same.

use dicebear_core::{Avatar, Style};
use scylla_domain::platform::images::{GlyphAvatar, GlyphPaint};
use serde_json::{json, Map, Value};

use crate::Error;

/// The style "Glyphs" of DiceBear 11 with no white base under the background, and the shape
/// `scylla`. The file is the same in scylla-web.
const STYLE: &str = include_str!("glyph_style.json");

/// Draws the SVG of a glyph avatar.
pub(crate) fn draw(avatar: &GlyphAvatar) -> Result<String, Error> {
    let style = Style::from_str(STYLE)
        .map_err(|error| Error::Internal(format!("the glyph style is not valid: {error}")))?;
    let drawn = Avatar::new(&style, options(avatar))
        .map_err(|error| Error::Internal(format!("the draw of a glyph avatar failed: {error}")))?;
    Ok(drawn.to_svg().to_owned())
}

/// Gives the options of DiceBear for a glyph avatar: each value is explicit, and the colors keep
/// their order.
pub(super) fn options(avatar: &GlyphAvatar) -> Value {
    let mut options = Map::new();
    options.insert("shapeVariant".into(), json!([avatar.shape.as_str()]));
    paint(&mut options, "glyphColor", &avatar.glyph);
    paint(&mut options, "backgroundColor", &avatar.background);
    options.insert("flip".into(), json!(avatar.flip.as_str()));
    options.insert("rotate".into(), json!(avatar.rotate));
    options.insert("scale".into(), json!(f64::from(avatar.scale) / 100.0));
    Value::Object(options)
}

/// Adds the options of one paint, with the prefix of its part.
fn paint(options: &mut Map<String, Value>, part: &str, paint: &GlyphPaint) {
    let colors: Vec<&str> = paint.colors.iter().map(|color| color.as_str()).collect();
    options.insert(part.into(), json!(colors));
    options.insert(format!("{part}Fill"), json!([paint.fill.as_str()]));
    options.insert(format!("{part}FillStops"), json!(colors.len().max(2)));
    options.insert(format!("{part}Angle"), json!(paint.angle));
    options.insert(format!("{part}Order"), json!("fixed"));
}
