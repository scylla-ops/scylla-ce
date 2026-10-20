set dotenv-load
set quiet
set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

DOCKER_USER  := env("DOCKER_USER", "godlyjaaaaj")
VERSION      := env("VERSION", "latest")
DATABASE_URL := env("DATABASE_URL", "postgres://scylla:scylla@localhost:5432/scylla")
BUILDER      := env("BUILDER", "scylla-builder")

# -- Aliases --
alias u := up
alias s := start
alias d := down
alias l := logs

# Show available commands
default:
    @just --list

# -- Dev (local stack) --

# Use the repo git hooks (.githooks) for this clone
[group('dev')]
hooks:
    git config core.hooksPath .githooks

# Run the workspace tests (the sqlx tests need `just db-up` first)
[group('dev')]
[no-exit-message]
test *args='--workspace':
    DATABASE_URL={{DATABASE_URL}} cargo test {{args}}

# A release `cargo build` compiles web/dist into the control-plane
# binary, so this has to run first when building outside Docker. The image does
# it on its own, in the Dockerfile's `ui` stage.
# Build the web UI (the scylla-web submodule) into web/dist (prerequisite of a native release build)
[group('dev')]
[no-exit-message]
ui-build:
    cd web && pnpm install --frozen-lockfile && pnpm run build

# Build all services for local dev (native arch)
[group('dev')]
local:
    docker compose build

# Start the stack from already-present images (no pull, no rebuild)
[group('dev')]
[no-exit-message]
start:
    docker compose up -d

# Pull the released beta images and start the stack
[group('dev')]
[no-exit-message]
up:
    docker compose pull
    docker compose up -d

# Stop all services
[group('dev')]
[no-exit-message]
down:
    docker compose down

# Show service logs (all or specific: just logs scylla-ce)
[group('dev')]
[no-exit-message]
logs *svc:
    docker compose logs -f {{svc}}

# Show running containers
[group('dev')]
[no-exit-message]
status:
    docker compose ps

# Remove this project's containers, networks, volumes, and locally-built images
[group('dev')]
[confirm("Remove Scylla containers, networks, volumes, and local images?")]
clean:
    docker compose down --rmi local --volumes --remove-orphans

# -- Database (sqlx) --

# Start only the Postgres dev DB (for running migrations / tests locally)
[group('db')]
db-up:
    docker compose up -d postgres

# Apply pending migrations against $DATABASE_URL (uses sqlx-cli)
[group('db')]
db-migrate:
    DATABASE_URL={{DATABASE_URL}} cargo sqlx migrate run --source migrations

# Revert the most recent migration
[group('db')]
db-revert:
    DATABASE_URL={{DATABASE_URL}} cargo sqlx migrate revert --source migrations

# Regenerate the offline query cache (commit the resulting .sqlx/ dir).
# --all-features so every feature-gated query stays in the cache.
[group('db')]
db-prepare:
    DATABASE_URL={{DATABASE_URL}} cargo sqlx prepare --workspace -- --all-features --tests

# Verify .sqlx/ is up-to-date
[group('db')]
db-prepare-check:
    DATABASE_URL={{DATABASE_URL}} cargo sqlx prepare --workspace --check -- --all-features --tests

# Drop & recreate the local Postgres dev volume (DESTRUCTIVE)
[group('db')]
[confirm("Drop scylla-postgres data volume?")]
db-reset:
    docker compose rm -sfv postgres
    docker volume rm scylla_postgres_data || true
    docker compose up -d postgres

# -- Release (multi-arch build & push to Docker Hub) --
#
# The build is declared in docker-bake.hcl; `latest` follows stable versions
# automatically. Full process, checklist and rollback: RELEASING.md

# One-time: create the multi-arch buildx builder
[group('release')]
release-setup:
    docker buildx inspect {{BUILDER}} >/dev/null 2>&1 || docker buildx create --name {{BUILDER}} --driver docker-container --bootstrap

# Print the resolved release plan without building anything
[group('release')]
[no-exit-message]
release-plan version:
    VERSION={{version}} docker buildx bake -f docker-bake.hcl --print release

# Build & push every release image (e.g. just release 0.4.0-beta)
[group('release')]
[no-exit-message]
release version: release-setup
    VERSION={{version}} GIT_SHA=`git rev-parse HEAD` docker buildx bake -f docker-bake.hcl --builder {{BUILDER}} --push release

# Show the published manifests, to confirm every platform is there
[group('release')]
[no-exit-message]
release-verify version:
    for image in scylla-ce scylla-agent; do docker buildx imagetools inspect {{DOCKER_USER}}/$image:{{version}}; done

# Point a channel tag at a published version, without rebuilding
[group('release')]
[no-exit-message]
release-promote version tag:
    for image in scylla-ce scylla-agent; do docker buildx imagetools create --tag {{DOCKER_USER}}/$image:{{tag}} {{DOCKER_USER}}/$image:{{version}}; done

# -- Protos --

# Lint every .proto against the buf STANDARD rules
[group('proto')]
[no-exit-message]
proto-lint:
    buf lint crates/scylla-proto/proto

# Rewrite every .proto in canonical buf formatting
[group('proto')]
[no-exit-message]
proto-fmt:
    buf format -w crates/scylla-proto/proto

# Fail if the submodule's schema breaks wire or source compatibility with the pin on main
[group('proto')]
[no-exit-message]
proto-breaking:
    buf breaking crates/scylla-proto/proto --against '.git#branch=main,subdir=crates/scylla-proto/proto,recurse_submodules=true'
