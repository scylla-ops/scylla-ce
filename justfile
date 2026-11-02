set dotenv-load

# Starts Postgres on 127.0.0.1:55432.
db-up:
    docker compose up -d --wait db

# Applies the core migrations with sqlx-cli, for the query! macros.
migrate: db-up
    cargo sqlx database create
    cargo sqlx migrate run --source scylla-core/migrations --config scylla-core/sqlx.toml

# Refreshes .sqlx/ after a SQL change.
prepare: migrate
    cargo sqlx prepare --workspace -- --all-targets

# Serves the Community edition. Also trusts the x-scylla-actor header: dev only.
run: migrate
    SCYLLA_TRUST_ACTOR_HEADER=true cargo run -p scylla-ce -- serve

# Runs all tests.
test: migrate
    cargo test --workspace

# Runs all checks of the finish-change skill.
lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    buf lint
    scripts/check-structure.sh

# Creates an actor of a kind and prints its id.
actor kind="dev":
    @cargo run -q -p scylla-ce -- actor create --kind {{kind}}

# Creates an API key for an actor and prints it one time.
key actor:
    @cargo run -q -p scylla-ce -- key create --actor {{actor}}
