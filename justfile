set dotenv-load

db-up:
    docker compose up -d --wait db

migrate: db-up
    sqlx database create
    sqlx migrate run --source scylla-core/migrations --config scylla-core/sqlx.toml

prepare: migrate
    cargo sqlx prepare --workspace -- --all-targets

run-ce: migrate
    cargo run -p scylla-ce

test: migrate
    cargo test --workspace

lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    buf lint
    scripts/check-structure.sh

user name:
    @docker compose exec -T db psql -U scylla -d scylla -qtAc "INSERT INTO users (id, name) VALUES (gen_random_uuid(), '{{name}}') RETURNING id"

measure:
    scripts/measure-build.sh
