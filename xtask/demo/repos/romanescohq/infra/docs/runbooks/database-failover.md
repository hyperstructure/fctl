# The database is overloaded or down

Alarm: `database-cpu`, the database's CPU above 80% for 15 minutes.

## First, look

1. Performance Insights for the instance, in the RDS console: which queries take the time.
2. #platform in chat: is a migration or a backfill running?

## Make it stop hurting

- A runaway query: `scripts/psql.sh production`, find its `pid` in `pg_stat_activity`, and `SELECT pg_cancel_backend(pid)`.
- The instance is down: production is Multi-AZ, so RDS fails over by itself in one to two minutes. Watch the API's error rate come back down; there's nothing to do unless it doesn't.

## After

Write down what happened in the incident log, and open an issue for whatever made it happen.
