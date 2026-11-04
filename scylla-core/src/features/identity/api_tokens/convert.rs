//! The conversions between the proto messages and the domain types of the API tokens.

use scylla_domain::identity::api_tokens::{ApiTokenInfo, TokenLimits, TokenPreset};
use scylla_domain::projects::ProjectId;
use scylla_proto::auth::v1 as pb;

use crate::api::parse::{self, ToProto};
use crate::Error;

impl ToProto<pb::ApiToken> for ApiTokenInfo {
    fn to_proto(self) -> pb::ApiToken {
        pb::ApiToken {
            id: Some(self.id.to_proto()),
            actor_id: Some(self.actor_id.to_proto()),
            name: self.name,
            hint: self.hint,
            created_by: Some(self.created_by.to_proto()),
            created_at: Some(parse::timestamp(self.created_at)),
            expires_at: Some(parse::timestamp(self.expires_at)),
            last_used_at: self.last_used_at.map(parse::timestamp),
            revoked_at: self.revoked_at.map(parse::timestamp),
            limits: Some(self.limits.to_proto()),
        }
    }
}

impl ToProto<pb::TokenLimits> for TokenLimits {
    fn to_proto(self) -> pb::TokenLimits {
        let preset = match self.preset {
            TokenPreset::ReadOnly => pb::TokenPreset::ReadOnly,
            TokenPreset::Deploy => pb::TokenPreset::Deploy,
            TokenPreset::Full => pb::TokenPreset::Full,
            TokenPreset::Custom => pb::TokenPreset::Custom,
        };
        pb::TokenLimits {
            preset: preset.into(),
            actions: self.actions,
            project_ids: self.projects.into_iter().map(ToProto::to_proto).collect(),
        }
    }
}

/// Reads the limits of a new token. No limits give the read-only preset.
pub(crate) fn limits(message: Option<pb::TokenLimits>) -> Result<TokenLimits, Error> {
    let message = message.unwrap_or_default();
    let preset = match pb::TokenPreset::try_from(message.preset) {
        Ok(pb::TokenPreset::Unspecified | pb::TokenPreset::ReadOnly) => TokenPreset::ReadOnly,
        Ok(pb::TokenPreset::Deploy) => TokenPreset::Deploy,
        Ok(pb::TokenPreset::Full) => TokenPreset::Full,
        Ok(pb::TokenPreset::Custom) => TokenPreset::Custom,
        Err(_) => return Err(Error::invalid("limits.preset", "must be a known value")),
    };
    let projects = message
        .project_ids
        .into_iter()
        .map(|id| parse::field::<_, ProjectId>(id, "limits.project_ids"))
        .collect::<Result<Vec<_>, _>>()?;
    TokenLimits::new(preset, message.actions, projects)
        .map_err(|(field, error)| Error::invalid(format!("limits.{field}"), error.to_string()))
}
