//! The database: a pool, and the queries more than one module needs.

use sqlx::PgPool;

#[derive(Clone)]
pub struct Db(PgPool);

impl Db {
    pub fn new(pool: PgPool) -> Self {
        Self(pool)
    }

    pub fn pool(&self) -> &PgPool {
        &self.0
    }
}

pub async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .connect(url)
        .await
}
