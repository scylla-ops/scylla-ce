# syntax=docker/dockerfile:1

# === Web UI ===
# --platform=$BUILDPLATFORM on purpose: the output is architecture-independent
# JavaScript, so a multi-arch `just release` builds it once natively instead of
# twice, the second time under qemu emulation.
FROM --platform=$BUILDPLATFORM node:22-alpine AS ui

# protoc on PATH so @protobuf-ts/protoc uses the system binary instead of
# downloading a release from GitHub at build time. The protos import
# google/protobuf/timestamp.proto; `protobuf-dev` ships the well-known types
# under /usr/include and protoc resolves them automatically.
RUN apk add --no-cache protobuf protobuf-dev
RUN corepack enable

ENV PNPM_HOME=/pnpm \
    PATH="/pnpm:/app/node_modules/.bin:$PATH"

WORKDIR /app

COPY web/package.json web/pnpm-lock.yaml web/pnpm-workspace.yaml ./

RUN --mount=type=cache,id=pnpm-store,target=/pnpm/store \
    pnpm config set store-dir /pnpm/store && \
    pnpm install --frozen-lockfile

COPY web/ .

# VITE_API_URL is deliberately unset. The transport falls back to a relative
# base URL, so the bundle talks to whatever origin served it — which is what
# stops an image from being pinned to one deployment's hostname.
RUN pnpm run build

# === Chef base ===
FROM rust:1-bookworm AS chef
# protobuf-compiler ships `protoc`; libprotobuf-dev ships the well-known-type
# .proto files under /usr/include/google/protobuf (e.g. timestamp.proto), which
# protoc auto-resolves. The protos import google/protobuf/timestamp.proto, so
# both are required — the compiler alone is not enough.
RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler libprotobuf-dev && \
    rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --locked
WORKDIR /app

# === Planner: extract dependency recipe from Cargo.toml/Cargo.lock ===
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# === Deps: cook the dependencies the builders will reuse ===
# Critically there is NO `ARG` in this stage: an in-scope ARG is folded into the
# RUN's cache key (BuildKit treats it as an implicit env prefix), which would
# give every service a distinct deps layer and silently un-share the cook.
# Cook the ENTIRE workspace once — service-independent, so the layer is SHARED
# across all service builds (ce, agent…).
FROM chef AS deps
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo chef cook --release --recipe-path recipe.json

# === Source layer, shared by both binaries ===
# ARGs live here rather than in `deps`, so the cooked layer above stays shared.
FROM deps AS src
COPY . .
ARG CARGO_BUILD_JOBS=2
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}
# Use the committed .sqlx/ offline cache so query!/query_as! macros expand
# without needing a live Postgres at build time.
ENV SQLX_OFFLINE=true

# One build stage per binary instead of a shared `ARG PACKAGE`: only the control
# plane (the `scylla-ce` package) needs the web UI, and giving it its own branch
# is what keeps the agent image from paying for the node stage. BuildKit only
# builds the stages its target actually reaches, so `--target scylla-agent`
# never runs `ui`.
FROM src AS build-agent
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo build --release -p scylla-agent && \
    cp target/release/scylla-agent /app/service

FROM src AS build-ce
# The SPA is compiled into the binary (rust-embed), so it has to land before
# cargo runs, at the path build.rs and the #[folder] attribute both expect.
COPY --from=ui /app/dist ./web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo build --release -p scylla-ce && \
    cp target/release/scylla-ce /app/service

# === Runtime ===
FROM debian:bookworm-slim AS runtime-base

LABEL org.opencontainers.image.source="https://github.com/scylla-ops/scylla"

# curl is for the control plane's HEALTHCHECK. It used to live on the Caddy
# image that served the frontend; that image is gone, and bookworm-slim ships
# neither curl nor wget.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 curl && \
    rm -rf /var/lib/apt/lists/*

# The agent's default --workspace-root lives under /var/lib/scylla. Bake the
# directory in with the right owner so a named volume mounted there inherits
# appuser ownership (Docker copies ownership from the image path on first
# mount) — without it the agent gets a root-owned dir and fails on permission.
RUN groupadd --gid 10001 appuser && \
    useradd --uid 10001 --gid appuser --no-create-home appuser && \
    mkdir -p /var/lib/scylla/workspaces && \
    chown -R appuser:appuser /var/lib/scylla

WORKDIR /app
USER appuser
ENTRYPOINT ["./service"]

# Final stages are named after their package, so `--target <package>` reads the
# same way the old `--build-arg PACKAGE=<package>` did.
FROM runtime-base AS scylla-agent
COPY --from=build-agent --chown=appuser:appuser /app/service ./service

FROM runtime-base AS scylla-ce
COPY --from=build-ce --chown=appuser:appuser /app/service ./service
# One port now: web UI, gRPC, gRPC-Web, reflection and webhook ingress.
EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/healthz || exit 1
