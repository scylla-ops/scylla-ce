//! The conversions of the custom icons, of their colors, of the avatars and of the glyph avatars
//! between the proto messages and the domain.

use scylla_domain::platform::images::{
    glyph_angle, Avatar, GlyphAvatar, GlyphColor, GlyphFill, GlyphFlip, GlyphName, GlyphPaint,
    GlyphShape, Icon, IconChange, IconColor, IconGlyph, Upload,
};
use scylla_domain::DomainError;
use scylla_proto::common::v1 as pb;
use scylla_proto::common::v1::icon::Kind;
use scylla_proto::common::v1::icon_change::Kind as ChangeKind;

use super::parse::{FromProto, Invalid, ToProto};
use crate::Error;

/// Reads a color of the web app or a hex color. An empty text is no color: the default color.
pub(crate) fn color(text: String, field: &'static str) -> Result<Option<IconColor>, Error> {
    IconColor::parse(text).map_err(|error| Error::invalid(field, error.to_string()))
}

impl ToProto<pb::Icon> for Icon {
    fn to_proto(self) -> pb::Icon {
        let (kind, glyph_avatar) = match self {
            Self::Glyph(glyph) => {
                let glyph = pb::IconGlyph {
                    name: glyph.name.as_str().to_owned(),
                    color: glyph
                        .color
                        .map(|color| color.as_str().to_owned())
                        .unwrap_or_default(),
                };
                (Kind::Glyph(glyph), None)
            }
            Self::Image(id) => (Kind::ImageId(id.to_proto()), None),
            Self::GlyphAvatar(id, glyph) => (Kind::ImageId(id.to_proto()), Some(glyph.to_proto())),
        };
        pb::Icon {
            kind: Some(kind),
            glyph_avatar,
        }
    }
}

impl ToProto<pb::Avatar> for Avatar {
    fn to_proto(self) -> pb::Avatar {
        pb::Avatar {
            image_id: self.image.map(ToProto::to_proto),
            color: self
                .color
                .map(|color| color.as_str().to_owned())
                .unwrap_or_default(),
            glyph_avatar: self.glyph.map(ToProto::to_proto),
        }
    }
}

impl ToProto<pb::GlyphAvatar> for GlyphAvatar {
    fn to_proto(self) -> pb::GlyphAvatar {
        let flip = match self.flip {
            GlyphFlip::None => pb::GlyphFlip::Unspecified,
            GlyphFlip::Horizontal => pb::GlyphFlip::Horizontal,
            GlyphFlip::Vertical => pb::GlyphFlip::Vertical,
            GlyphFlip::Both => pb::GlyphFlip::Both,
        };
        pb::GlyphAvatar {
            shape: self.shape.into(),
            glyph: Some(paint_to_proto(self.glyph)),
            background: Some(paint_to_proto(self.background)),
            flip: flip.into(),
            rotate: self.rotate,
            scale: self.scale as i32,
        }
    }
}

/// Writes a paint of a glyph avatar.
fn paint_to_proto(paint: GlyphPaint) -> pb::GlyphPaint {
    let fill = match paint.fill {
        GlyphFill::Solid => pb::GlyphFill::Unspecified,
        GlyphFill::Linear => pb::GlyphFill::Linear,
        GlyphFill::Radial => pb::GlyphFill::Radial,
    };
    pb::GlyphPaint {
        colors: paint.colors.into_iter().map(String::from).collect(),
        fill: fill.into(),
        angle: paint.angle,
    }
}

impl FromProto<pb::GlyphAvatar> for GlyphAvatar {
    fn from_proto(value: pb::GlyphAvatar) -> Result<Self, Invalid> {
        let shape = GlyphShape::parse(value.shape).map_err(|e| Invalid::at("shape", e))?;
        let glyph = value
            .glyph
            .ok_or(Invalid::at("glyph", DomainError::Required))?;
        let glyph = paint(glyph).map_err(|(part, e)| Invalid::at(glyph_field(part), e))?;
        let background = match value.background {
            Some(background) => {
                paint(background).map_err(|(part, e)| Invalid::at(background_field(part), e))?
            }
            None => GlyphPaint::white(),
        };
        let flip = match pb::GlyphFlip::try_from(value.flip) {
            Ok(pb::GlyphFlip::Unspecified) => GlyphFlip::None,
            Ok(pb::GlyphFlip::Horizontal) => GlyphFlip::Horizontal,
            Ok(pb::GlyphFlip::Vertical) => GlyphFlip::Vertical,
            Ok(pb::GlyphFlip::Both) => GlyphFlip::Both,
            Err(_) => {
                return Err(Invalid::at(
                    "flip",
                    DomainError::OutOfRange { min: 0, max: 3 },
                ))
            }
        };
        let rotate = glyph_angle(value.rotate).map_err(|e| Invalid::at("rotate", e))?;
        let scale = GlyphAvatar::scale(value.scale).map_err(|e| Invalid::at("scale", e))?;
        Ok(Self {
            shape,
            glyph,
            background,
            flip,
            rotate,
            scale,
        })
    }
}

/// The part of a paint that is not valid: its colors, its fill or its angle.
type PaintError = (&'static str, DomainError);

/// Reads a paint of a glyph avatar.
fn paint(value: pb::GlyphPaint) -> Result<GlyphPaint, PaintError> {
    let colors = value
        .colors
        .into_iter()
        .map(GlyphColor::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ("colors", e))?;
    let fill = match pb::GlyphFill::try_from(value.fill) {
        Ok(pb::GlyphFill::Unspecified) => GlyphFill::Solid,
        Ok(pb::GlyphFill::Linear) => GlyphFill::Linear,
        Ok(pb::GlyphFill::Radial) => GlyphFill::Radial,
        Err(_) => return Err(("fill", DomainError::OutOfRange { min: 0, max: 2 })),
    };
    GlyphPaint::new(colors, fill, value.angle).map_err(|e| match e {
        DomainError::NotAnAngle => ("angle", e),
        e => ("colors", e),
    })
}

/// Gives the path of a part of the paint of the glyph.
fn glyph_field(part: &str) -> &'static str {
    match part {
        "fill" => "glyph.fill",
        "angle" => "glyph.angle",
        _ => "glyph.colors",
    }
}

/// Gives the path of a part of the paint of the background.
fn background_field(part: &str) -> &'static str {
    match part {
        "fill" => "background.fill",
        "angle" => "background.angle",
        _ => "background.colors",
    }
}

/// Gives the path of a field of a glyph avatar inside an `IconChange`.
fn in_icon_change(field: &str) -> &'static str {
    match field {
        "shape" => "glyph_avatar.shape",
        "glyph" => "glyph_avatar.glyph",
        "glyph.fill" => "glyph_avatar.glyph.fill",
        "glyph.angle" => "glyph_avatar.glyph.angle",
        "glyph.colors" => "glyph_avatar.glyph.colors",
        "background.fill" => "glyph_avatar.background.fill",
        "background.angle" => "glyph_avatar.background.angle",
        "background.colors" => "glyph_avatar.background.colors",
        "flip" => "glyph_avatar.flip",
        "rotate" => "glyph_avatar.rotate",
        "scale" => "glyph_avatar.scale",
        _ => "glyph_avatar",
    }
}

impl FromProto<pb::IconChange> for IconChange {
    fn from_proto(value: pb::IconChange) -> Result<Self, Invalid> {
        match value.kind {
            None => Ok(Self::Default),
            Some(ChangeKind::Glyph(glyph)) => {
                let name =
                    GlyphName::parse(glyph.name).map_err(|e| Invalid::at("glyph.name", e))?;
                let color =
                    IconColor::parse(glyph.color).map_err(|e| Invalid::at("glyph.color", e))?;
                Ok(Self::Glyph(IconGlyph { name, color }))
            }
            Some(ChangeKind::Image(bytes)) => {
                let upload = Upload::parse(bytes).map_err(|e| Invalid::at("image", e))?;
                Ok(Self::Image(upload))
            }
            Some(ChangeKind::GlyphAvatar(avatar)) => GlyphAvatar::from_proto(avatar)
                .map(Self::GlyphAvatar)
                .map_err(|e| e.inside(in_icon_change)),
        }
    }
}
