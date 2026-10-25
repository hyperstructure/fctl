"""Two vans and a dozen stops from one depot, the README's example grown up."""

from milkfloat import TimeWindow, Vehicle, Visit, solve

HOUR = 3600
DEPOT = (59.3326, 18.0649)

vans = [Vehicle(f"van-{i}", depot=DEPOT, shift=TimeWindow(6 * HOUR, 14 * HOUR), capacity=[40]) for i in (1, 2)]
stops = [
    Visit(f"stop-{i}", (59.30 + i * 0.004, 18.02 + (i % 5) * 0.015), service=240, demand=[6])
    for i in range(12)
]

solution = solve(vans, stops, seconds=2, seed=1)
for route in solution.routes:
    print(route.vehicle, " → ".join(route.stops))
