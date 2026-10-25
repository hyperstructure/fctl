# plantwatch

Soil sensors on ESP32-C3 boards in every pot, Rust firmware on embassy, and a
dashboard on the mini PC under the TV that says which plant is thirsty.

- `firmware/`: wakes every 15 minutes, reads the soil sensor, publishes the
  reading over MQTT and goes back to deep sleep. One build per board:
  `BOARD=kitchen cargo run --release` flashes it over USB.
- `dashboard/`: listens for readings, keeps them in SQLite and serves the
  page at http://fika.local:8080.

## Boards

| Board | Pot | Sensor |
|---|---|---|
| `living-room` | Monstera | analog probe on GPIO2 |
| `kitchen` | Basil and thyme | analog probe on GPIO2 |
| `bedroom` | Snake plant | analog probe on GPIO2 |
| `balcony` | Tomatoes | analog probe on GPIO2 |

## Calibrating a pot

On the dashboard, open the pot and press Calibrate: Dry with the soil dry,
then Wet just after watering. The dashboard turns raw readings into percent
with those two.
