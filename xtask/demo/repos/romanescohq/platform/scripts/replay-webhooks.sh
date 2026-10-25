#!/usr/bin/env bash
# Replays a day's Stripe webhook log against a copy of the database, with as
# many job workers as you ask for: races between them show up only with two.
#
#   scripts/replay-webhooks.sh --workers 2 --log webhooks.jsonl [--db copy-of-prod]
set -euo pipefail
workers=1 log= db=platform_replay
while [ $# -gt 0 ]; do
  case "$1" in
    --workers) workers=$2; shift 2 ;;
    --log) log=$2; shift 2 ;;
    --db) db=$2; shift 2 ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
done
[ -n "$log" ] || { echo "say which log: --log FILE" >&2; exit 2; }
cargo run -q --bin replay -- --database "postgres:///$db" --workers "$workers" "$log"
