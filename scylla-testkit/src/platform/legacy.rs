//! A database of the old product next to the test database, for the tests of the import.

use scylla_core::features::platform::import::{self, Report};
use scylla_core::infra::secret::Sealer;
use scylla_domain::identity::actors::ActorId;
use sqlx::postgres::PgPool;
use sqlx::{AssertSqlSafe, Executor};

use crate::TestApp;

/// The schema of the old product, with its built-in roles.
const SCHEMA: &str = include_str!("fixtures/legacy_schema.sql");

/// The rows of the old product: three users, two organizations, a project, a secret, two
/// pipelines, and their grants.
const ROWS: &str = include_str!("fixtures/legacy_rows.sql");

/// The key in hex of the secrets of the old product.
const OLD_KEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";

/// A database of the old product with its real schema and the rows of the fixture.
pub struct Legacy {
    /// The pool on the old database.
    pub pool: PgPool,
    /// The name of the old database.
    name: String,
}

impl Legacy {
    /// Makes the old database next to the test database, with the rows of the fixture. The
    /// value of the secret is `s3cr3t`, sealed with the old key.
    pub async fn create(pool: &PgPool) -> Self {
        // Create the database and its schema.
        let name = format!("legacy_{}", ActorId::generate().to_db().simple());
        pool.execute(AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .await
            .expect("create the old database");
        let options = (*pool.connect_options()).clone().database(&name);
        let old = PgPool::connect_with(options)
            .await
            .expect("connect to the old database");
        sqlx::raw_sql(SCHEMA)
            .execute(&old)
            .await
            .expect("old schema");

        // Add the rows, with the secret sealed by the old key.
        sqlx::raw_sql(ROWS).execute(&old).await.expect("old rows");
        sqlx::query("UPDATE project_secrets SET encrypted_value = $1")
            .bind(Self::old_sealer().seal("s3cr3t"))
            .execute(&old)
            .await
            .expect("old secret");
        Self { pool: old, name }
    }

    /// Returns the key of the secrets of the old product.
    pub fn old_sealer() -> Sealer {
        Sealer::from_hex(OLD_KEY).expect("a valid key")
    }

    /// Imports the old database into the base of `app`, with the hooks of its edition, its key
    /// and the old key.
    pub async fn import_into(&self, app: &TestApp) -> Result<Report, scylla_core::Error> {
        let old_sealer = Self::old_sealer();
        import::import(
            &self.pool,
            &app.pool,
            app.access.as_ref(),
            app.hooks.as_ref(),
            app.sealer.as_deref(),
            Some(&old_sealer),
        )
        .await
    }

    /// Closes the old database, and deletes it.
    pub async fn delete(self, pool: &PgPool) {
        self.pool.close().await;
        let name = self.name;
        pool.execute(AssertSqlSafe(format!("DROP DATABASE {name} WITH (FORCE)")))
            .await
            .expect("drop the old database");
    }
}
