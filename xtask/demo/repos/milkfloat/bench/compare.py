"""Compares a night's run with the one before, and with milkfloat's bests.

    python3 compare.py BEFORE.csv AFTER.csv [--best ../milkfloat/data/best.csv]

Fewer vehicles is better; with as many, a shorter distance (by more than
0.05%) is. Prints what got better and worse, and what beats the best.
"""

import argparse
import csv


def load(path):
    with open(path, newline="") as f:
        return {r["instance"]: (int(r["vehicles"]), float(r["distance"])) for r in csv.DictReader(f)}


def better(a, b):
    """Whether result a beats result b."""
    if a[0] != b[0]:
        return a[0] < b[0]
    return a[1] < b[1] * (1 - 0.0005)


def main():
    p = argparse.ArgumentParser()
    p.add_argument("before")
    p.add_argument("after")
    p.add_argument("--best")
    args = p.parse_args()
    before, after = load(args.before), load(args.after)
    best = load(args.best) if args.best else {}
    up = [i for i in after if i in before and better(after[i], before[i])]
    down = [i for i in after if i in before and better(before[i], after[i])]
    beats = [i for i in after if i in best and better(after[i], best[i])]
    print(f"{len(after)} instances: {len(up)} better, {len(down)} worse, {len(after) - len(up) - len(down)} the same")
    for label, names, was in (("better", up, before), ("worse", down, before), ("beats data/best.csv", beats, best)):
        for i in names:
            print(f"  {label}: {i} {was.get(i)} -> {after[i]}")


if __name__ == "__main__":
    main()
