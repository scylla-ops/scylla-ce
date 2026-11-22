//! The proto messages of the legal settings, and the check of a change.

use std::collections::HashSet;

use scylla_domain::platform::legal::{
    self, DocumentChange, DocumentContent, DocumentKind, DocumentValues, LegalChange,
    LegalDocument, LegalSettings, Operator,
};
use scylla_domain::DomainError;
use scylla_proto::platform::v1 as pb;
use scylla_proto::platform::v1::legal_document::Content;

use crate::api::parse::{self, ToProto};
use crate::Error;

impl ToProto<pb::LegalInfo> for LegalSettings {
    fn to_proto(self) -> pb::LegalInfo {
        pb::LegalInfo {
            operator: Some(operator(self.operator)),
            privacy_contact: self.privacy_contact,
            documents: self.documents.into_iter().map(document).collect(),
        }
    }
}

/// Makes the proto of the operator.
fn operator(operator: Operator) -> pb::LegalOperator {
    pb::LegalOperator {
        name: operator.name,
        address: operator.address,
        registration: operator.registration,
        publication_director: operator.publication_director,
        contact: operator.contact,
        host: operator.host,
    }
}

/// Makes the proto of a document.
fn document(document: LegalDocument) -> pb::LegalDocument {
    let values = document.values;
    let content = match values.content {
        DocumentContent::Url(url) => Content::Url(url),
        DocumentContent::Markdown(text) => Content::Markdown(text),
    };
    pb::LegalDocument {
        kind: kind_to_proto(document.kind) as i32,
        title: values.title,
        content: Some(content),
        version: values.version,
        effective_date: values
            .effective_date
            .map(legal::date_text)
            .unwrap_or_default(),
        acceptance_required: values.acceptance_required,
        updated_at: Some(parse::timestamp(document.updated_at)),
        changes: values.changes,
    }
}

/// Reads and checks a change of the legal settings. A bad value gives the path of its field.
pub(super) fn change(request: pb::UpdateLegalSettingsRequest) -> Result<LegalChange, Error> {
    // Check the operator and the privacy contact.
    let operator = request
        .operator
        .map(|raw| {
            let raw = Operator {
                name: raw.name,
                address: raw.address,
                registration: raw.registration,
                publication_director: raw.publication_director,
                contact: raw.contact,
                host: raw.host,
            };
            Operator::parse(raw)
                .map_err(|(field, error)| invalid(format!("operator.{field}"), error))
        })
        .transpose()?;
    let privacy_contact = request
        .privacy_contact
        .map(|raw| legal::privacy_contact(&raw).map_err(|error| invalid("privacy_contact", error)))
        .transpose()?;

    // Check each document, and refuse a second document of one kind.
    let mut kinds = HashSet::new();
    let mut documents = Vec::with_capacity(request.documents.len());
    for (index, raw) in request.documents.into_iter().enumerate() {
        let path = |field: &str| format!("documents[{index}].{field}");
        let kind = kind_of_proto(raw.kind)
            .ok_or_else(|| Error::invalid(path("kind"), "must be a kind of legal document"))?;
        if !kinds.insert(kind) {
            return Err(invalid(path("kind"), DomainError::Duplicate));
        }
        let values = raw
            .content
            .map(|content| {
                let content = match content {
                    Content::Url(url) => DocumentContent::Url(url),
                    Content::Markdown(text) => DocumentContent::Markdown(text),
                };
                DocumentValues::new(
                    &raw.title,
                    content,
                    &raw.version,
                    &raw.effective_date,
                    raw.acceptance_required,
                    &raw.changes,
                )
                .map_err(|(field, error)| invalid(path(field), error))
            })
            .transpose()?;
        documents.push(DocumentChange { kind, values });
    }
    Ok(LegalChange {
        operator,
        privacy_contact,
        documents,
    })
}

/// Makes the error of a bad field.
fn invalid(field: impl Into<String>, error: DomainError) -> Error {
    Error::invalid(field, error.to_string())
}

/// Makes the proto of a kind.
fn kind_to_proto(kind: DocumentKind) -> pb::LegalDocumentKind {
    match kind {
        DocumentKind::LegalNotice => pb::LegalDocumentKind::LegalNotice,
        DocumentKind::Privacy => pb::LegalDocumentKind::Privacy,
        DocumentKind::Cookies => pb::LegalDocumentKind::Cookies,
        DocumentKind::Terms => pb::LegalDocumentKind::Terms,
        DocumentKind::TermsOfSale => pb::LegalDocumentKind::TermsOfSale,
        DocumentKind::Dpa => pb::LegalDocumentKind::Dpa,
        DocumentKind::Subprocessors => pb::LegalDocumentKind::Subprocessors,
    }
}

/// Reads the proto of a kind. The unspecified kind and an unknown number give `None`.
fn kind_of_proto(value: i32) -> Option<DocumentKind> {
    let kind = pb::LegalDocumentKind::try_from(value).ok()?;
    DocumentKind::ALL
        .into_iter()
        .find(|known| kind_to_proto(*known) == kind)
}
