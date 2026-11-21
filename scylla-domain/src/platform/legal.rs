//! The legal settings of the instance: its documents, its operator and its privacy contact.

use time::{Date, OffsetDateTime};

use super::Platform;
use crate::DomainError;

mod checks;

pub use checks::date_text;
use checks::{address, at, block, date, line, markdown};

/// The kind of a legal document. Each kind has at most one document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DocumentKind {
    /// The legal notice: the identity of the publisher and of the host.
    LegalNotice,
    /// The privacy policy.
    Privacy,
    /// The cookies and the browser storage.
    Cookies,
    /// The terms of service.
    Terms,
    /// The terms of sale.
    TermsOfSale,
    /// The data processing agreement.
    Dpa,
    /// The list of the sub-processors.
    Subprocessors,
}

impl DocumentKind {
    /// Each kind, in the order of the lists.
    pub const ALL: [Self; 7] = [
        Self::LegalNotice,
        Self::Privacy,
        Self::Cookies,
        Self::Terms,
        Self::TermsOfSale,
        Self::Dpa,
        Self::Subprocessors,
    ];

    /// Returns the name of the kind in the database and in the path of its page.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LegalNotice => "legal-notice",
            Self::Privacy => "privacy",
            Self::Cookies => "cookies",
            Self::Terms => "terms",
            Self::TermsOfSale => "terms-of-sale",
            Self::Dpa => "dpa",
            Self::Subprocessors => "subprocessors",
        }
    }

    /// Reads the name of a kind.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

/// Where the text of a document is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentContent {
    /// An http or https address: each link opens this page.
    Url(String),
    /// A text in Markdown, which the page of the kind in the web app shows.
    Markdown(String),
}

/// The values of a document that is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentValues {
    /// The title. Empty: the client shows the title of the kind.
    pub title: String,
    /// Where the text is.
    pub content: DocumentContent,
    /// The version, for example `1.0`.
    pub version: String,
    /// The date of effect.
    pub effective_date: Option<Date>,
    /// `true` when each user must accept this version.
    pub acceptance_required: bool,
}

impl DocumentValues {
    /// The most characters of a title.
    pub const TITLE_MAX: usize = 100;
    /// The most characters of a version.
    pub const VERSION_MAX: usize = 32;
    /// The most characters of an address.
    pub const URL_MAX: usize = 2048;
    /// The most bytes of a text in Markdown.
    pub const MARKDOWN_MAX: usize = 256 * 1024;

    /// Checks the values of a document. Returns the field and the error of a bad value.
    pub fn new(
        title: &str,
        content: DocumentContent,
        version: &str,
        effective_date: &str,
        acceptance_required: bool,
    ) -> Result<Self, (&'static str, DomainError)> {
        let content = match content {
            DocumentContent::Url(url) => DocumentContent::Url(address(&url).map_err(at("url"))?),
            DocumentContent::Markdown(text) => {
                DocumentContent::Markdown(markdown(text).map_err(at("markdown"))?)
            }
        };
        Ok(Self {
            title: line(title, Self::TITLE_MAX).map_err(at("title"))?,
            content,
            version: line(version, Self::VERSION_MAX).map_err(at("version"))?,
            effective_date: date(effective_date).map_err(at("effective_date"))?,
            acceptance_required,
        })
    }
}

/// A document that is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegalDocument {
    /// The kind.
    pub kind: DocumentKind,
    /// The values.
    pub values: DocumentValues,
    /// The time of the last change.
    pub updated_at: OffsetDateTime,
}

/// The version of a document that is on, and its rule of acceptance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentVersion {
    /// The version.
    pub version: String,
    /// `true` when each user must accept this version.
    pub acceptance_required: bool,
}

/// A change of one document: new values, or `None` to turn its kind off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentChange {
    /// The kind.
    pub kind: DocumentKind,
    /// The new values, or `None`.
    pub values: Option<DocumentValues>,
}

/// The organization or the person that runs the instance. Each field can be empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Operator {
    /// The legal name and the legal form.
    pub name: String,
    /// The registered address.
    pub address: String,
    /// The registration number, the share capital and the VAT number.
    pub registration: String,
    /// The director of publication.
    pub publication_director: String,
    /// An email or a phone number to contact the operator.
    pub contact: String,
    /// The name, the address and the phone number of the host of the servers.
    pub host: String,
}

impl Operator {
    /// The most characters of a field of one line.
    pub const LINE_MAX: usize = 200;
    /// The most characters of a field of more lines.
    pub const BLOCK_MAX: usize = 500;

    /// Checks each field. Returns the field and the error of a bad value.
    pub fn parse(raw: Self) -> Result<Self, (&'static str, DomainError)> {
        Ok(Self {
            name: line(&raw.name, Self::LINE_MAX).map_err(at("name"))?,
            address: block(&raw.address, Self::BLOCK_MAX).map_err(at("address"))?,
            registration: block(&raw.registration, Self::BLOCK_MAX).map_err(at("registration"))?,
            publication_director: line(&raw.publication_director, Self::LINE_MAX)
                .map_err(at("publication_director"))?,
            contact: line(&raw.contact, Self::LINE_MAX).map_err(at("contact"))?,
            host: block(&raw.host, Self::BLOCK_MAX).map_err(at("host"))?,
        })
    }
}

/// The privacy contact: an email address or a page, one line. Empty: none.
pub fn privacy_contact(raw: &str) -> Result<String, DomainError> {
    line(raw, 320)
}

/// The legal settings of the instance.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LegalSettings {
    /// The operator.
    pub operator: Operator,
    /// The privacy contact. Empty: none.
    pub privacy_contact: String,
    /// The documents that are on, in the order of the kinds.
    pub documents: Vec<LegalDocument>,
}

impl LegalSettings {
    /// Returns the document of a kind, if it is on.
    pub fn document(&self, kind: DocumentKind) -> Option<&LegalDocument> {
        self.documents.iter().find(|document| document.kind == kind)
    }
}

/// A change of the legal settings. A field that is `None` does not change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LegalChange {
    /// The new operator.
    pub operator: Option<Operator>,
    /// The new privacy contact.
    pub privacy_contact: Option<String>,
    /// The documents to change, one for each kind at most.
    pub documents: Vec<DocumentChange>,
}

crate::authz! {
    /// The routes on the legal settings of the instance. `GetLegalInfo` needs no token.
    extend Platform as LegalHolder {
        /// The route `UpdateLegalSettings`.
        permission update_legal_settings = nil;
    }
}

#[cfg(test)]
mod tests;
