//! The logic of the legal settings of the instance.

use scylla_domain::platform::legal::{DocumentKind, DocumentVersion, LegalChange, LegalSettings};
use scylla_extension::Tx;
use sqlx::PgPool;

use super::store;
use crate::Error;

/// Returns the legal settings of the instance.
pub(crate) async fn get(pool: &PgPool) -> Result<LegalSettings, Error> {
    let mut tx = pool.begin().await?;
    let settings = read(&mut tx).await?;
    tx.commit().await?;
    Ok(settings)
}

/// Returns the version of the document of a kind, or `None` when the kind is off.
pub(crate) async fn version(
    tx: &mut Tx<'_>,
    kind: DocumentKind,
) -> Result<Option<DocumentVersion>, Error> {
    Ok(store::version(tx, kind).await?)
}

/// Changes the legal settings of the instance, and returns them.
pub(super) async fn update(pool: &PgPool, change: LegalChange) -> Result<LegalSettings, Error> {
    // Write the operator, the privacy contact and each changed document.
    let mut tx = pool.begin().await?;
    if let Some(operator) = &change.operator {
        store::set_operator(&mut tx, operator).await?;
    }
    if let Some(contact) = &change.privacy_contact {
        store::set_privacy_contact(&mut tx, contact).await?;
    }
    for document in &change.documents {
        match &document.values {
            Some(values) => store::put_document(&mut tx, document.kind, values).await?,
            None => store::remove_document(&mut tx, document.kind).await?,
        }
    }

    // Read the new settings, and log the change.
    let settings = read(&mut tx).await?;
    tx.commit().await?;
    let kinds: Vec<&str> = change
        .documents
        .iter()
        .map(|doc| doc.kind.as_str())
        .collect();
    tracing::info!(
        event = "legal.settings_changed",
        operator = change.operator.is_some(),
        privacy_contact = change.privacy_contact.is_some(),
        documents = kinds.join(",")
    );
    Ok(settings)
}

/// Reads the legal settings in a transaction.
async fn read(tx: &mut Tx<'_>) -> Result<LegalSettings, sqlx::Error> {
    let (operator, privacy_contact) = store::settings(tx).await?;
    let documents = store::documents(tx).await?;
    Ok(LegalSettings {
        operator,
        privacy_contact,
        documents,
    })
}
