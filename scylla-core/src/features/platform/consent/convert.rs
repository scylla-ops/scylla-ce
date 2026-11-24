//! The proto messages of the consent, and the check of a record.

use scylla_domain::platform::consent::{
    Choice, ConsentAction, ConsentId, ConsentSource, NewConsentRecord, Purpose,
};
use scylla_proto::platform::v1 as pb;

use crate::api::parse;
use crate::Error;

/// Reads and checks a record. A bad value gives the path of its field.
pub(super) fn record(request: pb::RecordConsentRequest) -> Result<NewConsentRecord, Error> {
    // Read the consent, the action, the source and each choice.
    let action = action(request.action())?;
    let source = source(request.source())?;
    let consent: Option<ConsentId> = request
        .consent_id
        .map(|id| parse::field(id, "consent_id"))
        .transpose()?;
    let choices = request
        .choices
        .into_iter()
        .enumerate()
        .map(|(index, choice)| {
            let purpose = Purpose::parse(&choice.purpose).map_err(|error| {
                Error::invalid(format!("choices[{index}].purpose"), error.to_string())
            })?;
            Ok(Choice {
                purpose,
                granted: choice.granted,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;

    // Check the values together.
    NewConsentRecord::new(
        consent,
        &request.banner_version,
        &request.banner_hash,
        choices,
        action,
        source,
    )
    .map_err(|(field, error)| Error::invalid(field, error.to_string()))
}

/// Reads the action.
fn action(action: pb::ConsentAction) -> Result<ConsentAction, Error> {
    match action {
        pb::ConsentAction::AcceptAll => Ok(ConsentAction::AcceptAll),
        pb::ConsentAction::RejectAll => Ok(ConsentAction::RejectAll),
        pb::ConsentAction::Custom => Ok(ConsentAction::Custom),
        pb::ConsentAction::Withdraw => Ok(ConsentAction::Withdraw),
        pb::ConsentAction::Unspecified => Err(Error::invalid("action", "is required")),
    }
}

/// Reads the source.
fn source(source: pb::ConsentSource) -> Result<ConsentSource, Error> {
    match source {
        pb::ConsentSource::Banner => Ok(ConsentSource::Banner),
        pb::ConsentSource::Preferences => Ok(ConsentSource::Preferences),
        pb::ConsentSource::Account => Ok(ConsentSource::Account),
        pb::ConsentSource::Unspecified => Err(Error::invalid("source", "is required")),
    }
}
