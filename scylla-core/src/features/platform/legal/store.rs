//! The SQL of the `legal_settings` table (one row) and of the `legal_documents` table (one row for
//! each kind that is on).

use scylla_domain::platform::legal::{
    DocumentContent, DocumentKind, DocumentValues, DocumentVersion, LegalDocument, Operator,
};
use scylla_extension::Tx;
use time::{Date, OffsetDateTime};

/// A row of the documents.
struct DocumentRow {
    /// The name of the kind.
    kind: String,
    /// The title.
    title: String,
    /// The address, for a document that is a link.
    url: Option<String>,
    /// The text in Markdown, for a document that is a text.
    markdown: Option<String>,
    /// The version.
    version: String,
    /// The date of effect.
    effective_date: Option<Date>,
    /// `true` when each user must accept this version.
    acceptance_required: bool,
    /// The time of the last change.
    updated_at: OffsetDateTime,
}

impl DocumentRow {
    /// Makes the document, or `None` for a kind that this build does not know.
    fn document(self) -> Option<LegalDocument> {
        let kind = DocumentKind::parse(&self.kind)?;
        let content = match (self.url, self.markdown) {
            (Some(url), _) => DocumentContent::Url(url),
            (None, markdown) => DocumentContent::Markdown(markdown.unwrap_or_default()),
        };
        let values = DocumentValues {
            title: self.title,
            content,
            version: self.version,
            effective_date: self.effective_date,
            acceptance_required: self.acceptance_required,
        };
        Some(LegalDocument {
            kind,
            values,
            updated_at: self.updated_at,
        })
    }
}

/// Returns the operator and the privacy contact.
pub(super) async fn settings(tx: &mut Tx<'_>) -> Result<(Operator, String), sqlx::Error> {
    let row = sqlx::query!(
        "SELECT operator_name, operator_address, operator_registration, publication_director,
                operator_contact, operator_host, privacy_contact
         FROM legal_settings"
    )
    .fetch_one(&mut **tx)
    .await?;
    let operator = Operator {
        name: row.operator_name,
        address: row.operator_address,
        registration: row.operator_registration,
        publication_director: row.publication_director,
        contact: row.operator_contact,
        host: row.operator_host,
    };
    Ok((operator, row.privacy_contact))
}

/// Returns the documents that are on, in the order of the kinds.
pub(super) async fn documents(tx: &mut Tx<'_>) -> Result<Vec<LegalDocument>, sqlx::Error> {
    let rows = sqlx::query_as!(
        DocumentRow,
        "SELECT kind, title, url, markdown, version, effective_date, acceptance_required, updated_at
         FROM legal_documents"
    )
    .fetch_all(&mut **tx)
    .await?;
    let mut documents: Vec<LegalDocument> =
        rows.into_iter().filter_map(DocumentRow::document).collect();
    documents.sort_by_key(|document| document.kind);
    Ok(documents)
}

/// Returns the version of the document of a kind, or `None` when the kind is off.
pub(super) async fn version(
    tx: &mut Tx<'_>,
    kind: DocumentKind,
) -> Result<Option<DocumentVersion>, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT version, acceptance_required FROM legal_documents WHERE kind = $1",
        kind.as_str()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| DocumentVersion {
        version: row.version,
        acceptance_required: row.acceptance_required,
    }))
}

/// Replaces the operator.
pub(super) async fn set_operator(tx: &mut Tx<'_>, operator: &Operator) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE legal_settings
         SET operator_name = $1, operator_address = $2, operator_registration = $3,
             publication_director = $4, operator_contact = $5, operator_host = $6",
        operator.name,
        operator.address,
        operator.registration,
        operator.publication_director,
        operator.contact,
        operator.host,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Replaces the privacy contact.
pub(super) async fn set_privacy_contact(tx: &mut Tx<'_>, contact: &str) -> Result<(), sqlx::Error> {
    sqlx::query!("UPDATE legal_settings SET privacy_contact = $1", contact)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Writes the document of a kind, which replaces the old one.
pub(super) async fn put_document(
    tx: &mut Tx<'_>,
    kind: DocumentKind,
    values: &DocumentValues,
) -> Result<(), sqlx::Error> {
    let (url, markdown) = match &values.content {
        DocumentContent::Url(url) => (Some(url.as_str()), None),
        DocumentContent::Markdown(text) => (None, Some(text.as_str())),
    };
    sqlx::query!(
        "INSERT INTO legal_documents
             (kind, title, url, markdown, version, effective_date, acceptance_required)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (kind) DO UPDATE
         SET title = excluded.title, url = excluded.url, markdown = excluded.markdown,
             version = excluded.version, effective_date = excluded.effective_date,
             acceptance_required = excluded.acceptance_required, updated_at = now()",
        kind.as_str(),
        values.title,
        url,
        markdown,
        values.version,
        values.effective_date,
        values.acceptance_required,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Removes the document of a kind: the kind is off.
pub(super) async fn remove_document(
    tx: &mut Tx<'_>,
    kind: DocumentKind,
) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM legal_documents WHERE kind = $1", kind.as_str())
        .execute(&mut **tx)
        .await?;
    Ok(())
}
