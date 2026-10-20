use crate::domain::errors::{DomainError, DomainResult};
use std::fmt::Display;

// Field names only: constraint names and values must not reach the client.
const UNIQUE_MESSAGES: &[(&str, &str)] = &[
    ("users_username_key", "Username already exists"),
    ("users_email_key", "Email already exists"),
    (
        "apps_organization_id_name_key",
        "App name already exists in this organization",
    ),
    (
        "app_secrets_app_id_label_key",
        "App secret label already exists on this app",
    ),
    (
        "project_secrets_project_id_name_key",
        "Secret name already exists in this project",
    ),
    (
        "pipeline_triggers_pipeline_id_name_key",
        "Trigger name already exists on this pipeline",
    ),
    ("roles_name_key", "Role name already exists"),
];

pub(crate) fn map_sqlx(err: sqlx::Error) -> DomainError {
    match err {
        sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
            tracing::debug!(error = %db_err, "unique-constraint violation");
            let message = db_err
                .constraint()
                .and_then(|c| UNIQUE_MESSAGES.iter().find(|(name, _)| *name == c))
                .map_or("a resource with these values already exists", |(_, m)| m);
            DomainError::conflict(message)
        }
        sqlx::Error::Database(db_err) if db_err.is_foreign_key_violation() => {
            tracing::debug!(error = %db_err, "foreign-key violation");
            DomainError::missing_reference()
        }
        sqlx::Error::Database(db_err) if db_err.code().is_some_and(|c| c.starts_with("22")) => {
            tracing::warn!(error = %db_err, "data exception");
            DomainError::validation("a value holds a character or size the database cannot store")
        }
        other => {
            tracing::error!(error = %other, "unexpected database error");
            DomainError::infrastructure("database error")
        }
    }
}

pub(crate) fn map_sqlx_not_found(
    err: sqlx::Error,
    entity_type: &str,
    id: impl Display,
) -> DomainError {
    match err {
        sqlx::Error::RowNotFound => DomainError::not_found(entity_type, id),
        other => map_sqlx(other),
    }
}

pub(crate) trait SqlxResultExt<T> {
    fn not_found_as(self, entity_type: &'static str, id: impl Display) -> DomainResult<T>;

    fn to_domain(self) -> DomainResult<T>;
}

impl<T> SqlxResultExt<T> for Result<T, sqlx::Error> {
    fn not_found_as(self, entity_type: &'static str, id: impl Display) -> DomainResult<T> {
        self.map_err(|e| map_sqlx_not_found(e, entity_type, id))
    }

    fn to_domain(self) -> DomainResult<T> {
        self.map_err(map_sqlx)
    }
}

pub(crate) trait DbFieldExt<T> {
    fn db_field(self, field: &'static str) -> DomainResult<T>;
}

impl<T, E: Display> DbFieldExt<T> for Result<T, E> {
    fn db_field(self, field: &'static str) -> DomainResult<T> {
        self.map_err(|e| DomainError::infrastructure(format!("invalid {field} in DB: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::{UNIQUE_MESSAGES, map_sqlx};
    use crate::domain::errors::DomainError;
    use sqlx::PgPool;

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_data_exception_is_the_client_value(pool: PgPool) {
        let err = sqlx::query("SELECT $1::text")
            .bind("a\0b")
            .fetch_one(&pool)
            .await
            .unwrap_err();
        assert!(matches!(map_sqlx(err), DomainError::Validation(_)));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn every_unique_message_names_an_existing_index(pool: PgPool) {
        for (name, _) in UNIQUE_MESSAGES {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE indexname = $1 AND indexdef LIKE 'CREATE UNIQUE%')",
            )
            .bind(name)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert!(exists, "{name}");
        }
    }
}
