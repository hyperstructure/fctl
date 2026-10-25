//! The platform's API: orders, subscriptions, farms and delivery routes.

mod billing;
mod db;
mod depots;
mod export;
mod jobs;
mod routes;
mod slots;
mod stripe;
mod webhooks;

use axum::Router;
use axum::routing::{get, post};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().json().init();
    let pool = db::connect(&std::env::var("DATABASE_URL")?).await?;
    sqlx::migrate!().run(&pool).await?;
    let app = Router::new()
        .route("/v1/slots/{postcode}", get(slots::for_postcode))
        .route("/v1/routes/{day}", post(routes::plan_day))
        .route("/webhooks/stripe", post(webhooks::stripe))
        .with_state(db::Db::new(pool.clone()));
    tokio::spawn(jobs::run(pool.clone()));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    axum::serve(listener, app).await?;
    Ok(())
}
