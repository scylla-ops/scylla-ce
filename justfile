set dotenv-load

# Starts Postgres (127.0.0.1:55432), SpiceDB (127.0.0.1:55051), the SpiceDB of the tests
# (127.0.0.1:55052, an empty store for each key) and Mailpit (SMTP 127.0.0.1:51025, web
# http://127.0.0.1:58025).
db-up:
    docker compose up -d --wait db spicedb spicedb-test mailpit

# Applies the core migrations with sqlx-cli, for the query! macros.
migrate: db-up
    cargo sqlx database create
    cargo sqlx migrate run --source scylla-core/migrations --config scylla-core/sqlx.toml

# Refreshes .sqlx/ after a SQL change.
prepare: migrate
    cargo sqlx prepare --workspace -- --all-targets

# Applies the migrations and the SpiceDB schema with the binary.
schema: migrate
    cargo run -q -p scylla-ce -- migrate

# Serves the Community edition with the dev keys of .env.
run: schema
    cargo run -p scylla-ce -- serve

# Runs all tests.
test: migrate
    cargo test --workspace

# Runs all checks of the finish-change skill.
lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    buf lint
    scripts/check-structure.sh

# Creates an active user with its personal workspace and prints its id. The password comes from stdin.
user email name:
    @cargo run -q -p scylla-ce -- user create --email {{email}} --name "{{name}}" --password-stdin

# Signs a short-lived token for an actor, for a first admin or a test.
token actor:
    @cargo run -q -p scylla-ce -- token create --actor {{actor}}
