import pytest

from milkfloat import TimeWindow, Vehicle, Visit, solve

HOUR = 3600
DEPOT = (59.3326, 18.0649)


def van(name="van-1", capacity=40):
    return Vehicle(name, depot=DEPOT, shift=TimeWindow(6 * HOUR, 14 * HOUR), capacity=[capacity])


def test_every_visit_gets_a_route():
    visits = [Visit(f"v{i}", (59.33 + i * 0.01, 18.06), service=60, demand=[5]) for i in range(6)]
    solution = solve([van(), van("van-2")], visits, seconds=1, seed=1)
    assert sorted(s for r in solution.routes for s in r.stops if s != "depot") == sorted(v.id for v in visits)
    assert solution.unassigned == []


def test_a_hard_window_is_kept():
    visit = Visit("bakery", (59.34, 18.05), window=TimeWindow(7 * HOUR, 7 * HOUR + 900), service=120)
    route = solve([van()], [visit], seconds=1, seed=1).routes[0]
    arrival = route.arrivals[route.stops.index("bakery")]
    assert 7 * HOUR <= arrival <= 7 * HOUR + 900


def test_no_vehicles_is_an_error():
    with pytest.raises(ValueError, match="no vehicles"):
        solve([], [Visit("a", (59.34, 18.05))], seconds=1)
