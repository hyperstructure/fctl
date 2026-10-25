# milkfloat

A vehicle-routing solver in Rust, with Python bindings: time windows,
capacities in several dimensions, and re-planning from where the vans are
while they're out. It plans the morning rounds of a few delivery companies,
and it's MIT licensed.

```sh
pip install milkfloat     # or: cargo add milkfloat
```

## An example

Two vans, five stops, everything in seconds after midnight in the depot's
time zone:

```python
from milkfloat import TimeWindow, Vehicle, Visit, solve

HOUR = 3600
depot = (59.3326, 18.0649)
vans = [
    Vehicle("van-1", depot=depot, shift=TimeWindow(6 * HOUR, 14 * HOUR), capacity=[40]),
    Vehicle("van-2", depot=depot, shift=TimeWindow(6 * HOUR, 14 * HOUR), capacity=[40]),
]
stops = [
    Visit("a", (59.3420, 18.0490), window=TimeWindow(7 * HOUR, 9 * HOUR), service=300, demand=[12]),
    Visit("b", (59.3175, 18.0755), window=TimeWindow(7 * HOUR, 8 * HOUR), service=300, demand=[8]),
    Visit("c", (59.3290, 18.0920), service=180, demand=[15]),
    Visit("d", (59.3510, 18.0610), window=TimeWindow(8 * HOUR, 10 * HOUR), service=300, demand=[20]),
    Visit("e", (59.3110, 18.0300), service=240, demand=[10]),
]
solution = solve(vans, stops, seconds=5, seed=1)
for route in solution.routes:
    print(route.vehicle, route.stops)
```

## What it does

- Hard and soft time windows.
- Capacities in as many dimensions as you need: crates, kilos, cold boxes.
- Re-planning: give it where each van is and what it has done, and it plans
  the rest of the day.
- Routes as GeoJSON, for a map.

## Benchmarks

Every night, [milkfloat/bench](https://github.com/milkfloat/bench) runs the
56 Solomon and 300 Gehring–Homberger instances against `main`; the results
are at [milkfloat.dev/bench](https://milkfloat.dev/bench). `data/best.csv`
holds milkfloat's best result on each.

## Developing

```sh
cargo test                                  # the solver
maturin develop --release && uv run pytest tests/python   # the bindings
uv run mkdocs serve                         # the docs
```
