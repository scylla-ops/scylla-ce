//! The conversions of the custom icons between the proto messages and the domain.

use scylla_domain::platform::images::{GlyphName, Icon, IconChange, IconColor, IconGlyph, Upload};
use scylla_proto::common::v1 as pb;
use scylla_proto::common::v1::icon::Kind;
use scylla_proto::common::v1::icon_change::Kind as ChangeKind;

use super::parse::{FromProto, Invalid, ToProto};

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
