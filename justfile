set dotenv-load

# Starts Postgres (127.0.0.1:55432), SpiceDB (127.0.0.1:55051) and the SpiceDB of the tests
# (127.0.0.1:55052, an empty store for each key).
db-up:
    docker compose up -d --wait db spicedb spicedb-test

# Starts the dev identity provider (Keycloak on 127.0.0.1:58080, user alice, password alice).
keycloak:
    docker compose --profile oidc up -d keycloak

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

# Creates an actor of a kind and prints its id.
actor kind="dev":
    @cargo run -q -p scylla-ce -- actor create --kind {{kind}}

# Signs a short-lived token for an actor, for a first admin or a test.
token actor:
    @cargo run -q -p scylla-ce -- token create --actor {{actor}}
