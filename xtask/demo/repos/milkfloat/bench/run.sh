#!/bin/sh
# Builds milkfloat at a commit and solves every instance with it.
#   ./run.sh --milkfloat DIR [--commit SHA] [--seconds 10] [--out results/tonight.csv]
set -eu
milkfloat=../milkfloat commit=main seconds=10 out=results/tonight.csv
while [ $# -gt 0 ]; do
  case "$1" in
    --milkfloat) milkfloat=$2; shift 2 ;;
    --commit) commit=$2; shift 2 ;;
    --seconds) seconds=$2; shift 2 ;;
    --out) out=$2; shift 2 ;;
    *) echo "run.sh: unknown option $1" >&2; exit 2 ;;
  esac
done
git -C "$milkfloat" fetch -q origin "$commit"
git -C "$milkfloat" checkout -q --detach FETCH_HEAD
cargo build -q --release --manifest-path "$milkfloat/Cargo.toml" --example bench
echo "instance,vehicles,distance,seconds" > "$out"
ls instances/solomon/*.txt instances/gehring-homberger/*.txt |
  xargs -P "$(nproc)" -I{} "$milkfloat/target/release/examples/bench" --instance {} --seconds "$seconds" --seed 1 --csv >> "$out"
sort -t, -k1,1 -o "$out" "$out"
