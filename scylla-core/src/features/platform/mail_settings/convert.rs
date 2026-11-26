//! The proto messages of the mail settings, and the check of a change.

use scylla_domain::platform::mail::{self, MailRelay, MailSecurity, RawMailRelay};
use scylla_domain::DomainError;
use scylla_proto::platform::v1 as pb;

use super::service::{Password, Settings, TestOutcome};
use crate::api::parse::ToProto;
use crate::infra::vcs::Hidden;
use crate::Error;

impl ToProto<pb::MailSettings> for Settings {
    fn to_proto(self) -> pb::MailSettings {
        pb::MailSettings {
            relay: self
                .relay
                .map(|(relay, password_set)| relay_to_proto(relay, password_set)),
            default_host: self.default_host.unwrap_or_default(),
        }
    }
}

impl ToProto<pb::SendTestMailResponse> for TestOutcome {
    fn to_proto(self) -> pb::SendTestMailResponse {
        match self.result {
            Ok(receipt) => pb::SendTestMailResponse {
                accepted: true,
                host: self.host,
                reply: receipt.reply.unwrap_or_default(),
                message_id: receipt.message_id.unwrap_or_default(),
                error: String::new(),
            },
            Err(error) => pb::SendTestMailResponse {
                host: self.host,
                error,
                ..pb::SendTestMailResponse::default()
            },
        }
    }
}

/// Reads and checks a change of the relay. A bad value gives the name of its field.
pub(super) fn change(
    request: pb::UpdateMailSettingsRequest,
) -> Result<(MailRelay, Password), Error> {
    let raw = RawMailRelay {
        host: request.host,
        port: request.port,
        security: security_of_proto(request.security),
        username: request.username,
        from_address: request.from_address,
        from_name: request.from_name,
    };
    let relay = MailRelay::parse(raw).map_err(|(field, error)| invalid(field, error))?;
    let password = match request.password.as_deref() {
        None => Password::Keep,
        Some("") => Password::Remove,
        Some(raw) => {
            let checked = mail::password(raw).map_err(|error| invalid("password", error))?;
            Password::Replace(Hidden::new(checked))
        }
    };
    Ok((relay, password))
}

/// Makes the proto of a relay.
fn relay_to_proto(relay: MailRelay, password_set: bool) -> pb::MailRelay {
    pb::MailRelay {
        host: relay.host,
        port: u32::from(relay.port),
        security: security_to_proto(relay.security) as i32,
        username: relay.username,
        password_set,
        from_address: relay.from_address,
        from_name: relay.from_name,
    }
}

/// Makes the proto of a security.
fn security_to_proto(security: MailSecurity) -> pb::MailSecurity {
    match security {
        MailSecurity::Tls => pb::MailSecurity::Tls,
        MailSecurity::StartTls => pb::MailSecurity::Starttls,
        MailSecurity::None => pb::MailSecurity::None,
    }
}

/// Reads the proto of a security. The unspecified security and an unknown number give `None`.
fn security_of_proto(value: i32) -> Option<MailSecurity> {
    match pb::MailSecurity::try_from(value).ok()? {
        pb::MailSecurity::Tls => Some(MailSecurity::Tls),
        pb::MailSecurity::Starttls => Some(MailSecurity::StartTls),
        pb::MailSecurity::None => Some(MailSecurity::None),
        pb::MailSecurity::Unspecified => None,
    }
}

/// Makes the error of a bad field.
fn invalid(field: &str, error: DomainError) -> Error {
    Error::invalid(field, error.to_string())
}
