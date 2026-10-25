# milkfloat/bench

Nightly benchmarks of [milkfloat](https://github.com/milkfloat/milkfloat)'s
`main`: the 56 Solomon instances (100 customers) and the 300
Gehring–Homberger ones (200 to 1,000), 10 seconds each, seed 1.

- `run.sh` builds milkfloat at a commit and solves every instance.
- `compare.py` compares a run with the night before and with milkfloat's
  `data/best.csv`.
- `results/latest.csv` is the latest night's run; `results/history.csv` has a
  line for every night. milkfloat.dev/bench shows both.

A run takes about an hour on 16 threads:

```sh
./run.sh --milkfloat ../milkfloat --seconds 10 --out results/tonight.csv
python3 compare.py results/latest.csv results/tonight.csv --best ../milkfloat/data/best.csv
```

## Latest

| | |
|---|---|
| milkfloat | `9e04b7a` |
| Instances | 356 |
| Better than the night before | 0 |
| Worse | 0 |
