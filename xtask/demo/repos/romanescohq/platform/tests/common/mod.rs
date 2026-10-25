//! What the integration tests share: a database, migrated.

use sqlx::PgPool;

/// A test's database.
pub struct TestDb {
    pub pool: PgPool,
}

impl TestDb {
    /// The shared test database, migrated once: quick, for tests that only
    /// read and write their own rows.
    pub async fn shared() -> Self {
        Self { pool: platform_test::shared_pool().await }
    }

    /// A database of the test's own, created from the template: for tests
    /// that run jobs, which hold locks the others would wait for.
    pub async fn isolated() -> Self {
        Self { pool: platform_test::fresh_pool().await }
    }
}
