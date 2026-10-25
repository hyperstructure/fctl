mod db;
mod page;
mod thirst;

use anyhow::Context;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde::Deserialize;
use std::sync::{Arc, Mutex};

/// A board's reading, as the firmware publishes it.
#[derive(Deserialize)]
struct Reading {
    board: String,
    raw: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let db = Arc::new(Mutex::new(db::open("plantwatch.db")?));
    let (client, mut events) = AsyncClient::new(MqttOptions::new("dashboard", "localhost", 1883), 16);
    client.subscribe("plantwatch/readings", QoS::AtLeastOnce).await?;

    let readings = db.clone();
    tokio::spawn(async move {
        while let Ok(event) = events.poll().await {
            let Event::Incoming(Packet::Publish(p)) = event else { continue };
            let Ok(r) = serde_json::from_slice::<Reading>(&p.payload) else { continue };
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            if let Ok(db) = readings.lock() {
                let _ = db::record(&db, &r.board, r.raw, now);
            }
        }
    });

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    axum::serve(listener, page::router(db)).await.context("serving the dashboard")
}
