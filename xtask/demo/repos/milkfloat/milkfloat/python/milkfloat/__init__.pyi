from collections.abc import Sequence

class TimeWindow:
    """Seconds after midnight in the depot's time zone. Soft with a penalty
    per second late; hard without one."""

    start: int
    end: int
    penalty: float | None
    def __init__(self, start: int, end: int, penalty: float | None = None) -> None: ...

class Visit:
    id: str
    def __init__(
        self,
        id: str,
        location: tuple[float, float],
        *,
        window: TimeWindow | None = None,
        service: int = 0,
        demand: Sequence[int] = (),
    ) -> None: ...

class Vehicle:
    id: str
    def __init__(
        self,
        id: str,
        depot: tuple[float, float],
        shift: TimeWindow,
        capacity: Sequence[int],
    ) -> None: ...

class Route:
    vehicle: str
    stops: list[str]
    arrivals: list[int]

class Solution:
    routes: list[Route]
    unassigned: list[str]
    cost: float

def solve(
    vehicles: Sequence[Vehicle],
    visits: Sequence[Visit],
    *,
    seconds: float = 10.0,
    seed: int | None = None,
) -> Solution: ...
