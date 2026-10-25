# Romanesco's platform

Orders, subscriptions, farms and delivery routes behind every app: Rust (axum, sqlx)
on Postgres, with milkfloat planning the vans' routes.

```sh
docker compose up -d postgres
cargo test
```

CI builds with `SQLX_OFFLINE=true` from the query data in `.sqlx/`.
