# Scylla — architecture

Repo-wide map of the codebase. Each area keeps its own guide next to the code;
this file only says where to look.

## Repository shape

`binaries/` holds the packages that produce an executable, `crates/` the
libraries they link.

| Path | What it is |
|---|---|
| `crates/scylla-extension` | the edition boundary: the action pipeline (commands, stages, `Hooks`); depends on `scylla-domain` only. Guide: [`crates/scylla-extension/ARCHITECTURE.md`](./crates/scylla-extension/ARCHITECTURE.md) |
| `crates/scylla-domain` | dependency-light shared kernel (domain model, `JobEvent`) |
| `crates/scylla-proto` | the wire contract: the Rust bindings and their conversions; the protos are the `scylla-protos` git submodule in `proto/scylla/<domain>/v1/` |
| `crates/scylla-auth` | the access model: RBAC ports and types, the Cedar adapter |
| `crates/scylla-core` | use cases and ports, gRPC + HTTP surfaces, config, in-memory adapters |
| `crates/scylla-db` | the Postgres adapters, the pool, the embedded migrations |
| `crates/scylla-server` | the composition root: the `Server` builder (hook extensions, extra gRPC/HTTP services) and the `cli` every edition binary shares |
| `binaries/scylla-ce` | the Community Edition binary: a `main.rs` and the config files |
| `binaries/scylla-agent` | the worker installed per machine |
| `web` | the `scylla-web` git submodule: the web UI, compiled into the `scylla-ce` binary through `scylla-core` |

Dependencies point one way: `domain <- auth <- core <- db <- server <- ce`, with
`extension` between `domain` and `core`. A private Enterprise repo depends on
this one by git tag and registers its own hooks; nothing here depends on it.

Every package sits exactly two directories below the root, and for two of
them that depth is load-bearing: `sqlx::migrate!("../../migrations")`
(scylla-db) and the rust-embed `#[folder = "../../web/dist/"]`
(scylla-core) resolve against `CARGO_MANIFEST_DIR`.

## Guides

| Topic | Where |
|---|---|
| **Publishing release images** | [`RELEASING.md`](./RELEASING.md) |
| The action pipeline (commands, stages, hooks) | [`crates/scylla-extension/ARCHITECTURE.md`](./crates/scylla-extension/ARCHITECTURE.md) |
| Domain vocabulary | [`GLOSSARY.md`](./GLOSSARY.md) |
| Running the stack | [`README.md`](./README.md) |

## Conventions worth knowing before editing

- **Commands live in the `justfile`, logic does not.** `just --list` is the
  index. Recipes stay one-liners that delegate; a recipe growing an `if` or a
  `case` is a sign the logic belongs in the tool it calls.
- **Container builds live in `docker-bake.hcl`.** Tags included: they are
  derived from `VERSION` and `LATEST` in the bake file, not assembled by a
  wrapper. See [`RELEASING.md`](./RELEASING.md).
- **The backend builds offline.** `SQLX_OFFLINE=true` against the committed
  `.sqlx/` cache; after touching a `query!`/`query_as!` macro, run
  `just db-prepare` and commit the result.
- **The tests need Postgres.** The `#[sqlx::test]` tests in `scylla-db` use a
  live database. Run `just db-up`, then `just test`.
- **Protos are linted and breaking-checked.** `just proto-lint`,
  `just proto-fmt`, `just proto-breaking`.
- **The protos are a git submodule.** `crates/scylla-proto/proto` is
  [`scylla-ops/scylla-protos`](https://github.com/scylla-ops/scylla-protos).
  To change a proto: commit and push in the submodule first, then commit the
  new pin here. A new file must also go in `crates/scylla-proto/build.rs`.
- **The web UI is a git submodule.** `web/` is
  [`scylla-ops/scylla-web`](https://github.com/scylla-ops/scylla-web), with its
  own guide and CI. Only `just ui-build` and the Docker `ui` stage use it here.
