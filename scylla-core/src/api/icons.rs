//! The conversions of the custom icons, of their colors and of the avatars between the proto
//! messages and the domain.

use scylla_domain::platform::images::{
    Avatar, GlyphName, Icon, IconChange, IconColor, IconGlyph, Upload,
};
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
        let kind = match self {
            Self::Glyph(glyph) => Kind::Glyph(pb::IconGlyph {
                name: glyph.name.as_str().to_owned(),
                color: glyph
                    .color
                    .map(|color| color.as_str().to_owned())
                    .unwrap_or_default(),
            }),
            Self::Image(id) => Kind::ImageId(id.to_proto()),
        };
        pb::Icon { kind: Some(kind) }
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
        }
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
        }
    }
}
