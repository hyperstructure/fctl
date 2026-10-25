# Getting started

```sh
pip install milkfloat
```

A problem is vehicles and the visits they share out. Times are seconds
after midnight in the depot's time zone; locations are latitude and
longitude.

```python
from milkfloat import TimeWindow, Vehicle, Visit, solve

van = Vehicle("van-1", depot=(59.3326, 18.0649), shift=TimeWindow(21600, 50400), capacity=[40])
visits = [Visit("a", (59.3420, 18.0490), service=300, demand=[12])]
print(solve([van], visits, seconds=2).routes[0].stops)
```

`solve` searches for `seconds` and returns the best solution it found.
Give it a `seed` to get the same answer every time.
