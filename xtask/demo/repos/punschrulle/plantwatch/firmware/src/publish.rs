//! Wi-Fi, and the reading to the dashboard's MQTT broker on fika.

use crate::board::Board;
use embassy_net::Stack;
use serde::Serialize;

const BROKER: &str = "fika.local";
const TOPIC: &str = "plantwatch/readings";

#[derive(Serialize)]
struct Reading<'a> {
    board: &'a str,
    raw: u16,
}

/// Publishes one reading, the board's raw value.
pub async fn reading(stack: Stack<'static>, board: &Board, raw: u16) -> Result<(), Error> {
    let mut buf = [0u8; 96];
    let len = serde_json_core::to_slice(&Reading { board: board.id, raw }, &mut buf)
        .map_err(|_| Error::TooLong)?;
    mqtt_publish(stack, BROKER, TOPIC, &buf[..len]).await
}
