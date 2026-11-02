# Scylla open core POC: core and Community edition

This repository holds the core of Scylla and the Community edition binary `scylla-ce`. It is the public repository. The Enterprise edition is in a separate private repository (`../ee` in this POC). That repository uses the crates of this repository and adds its features through the extension points of `scylla-extension`.

The business domain is small on purpose: projects that belong to owners. The API is gRPC (tonic).

## Dependency direction

```
scylla-ce ──▶ scylla-core ──▶ scylla-extension ──▶ scylla-domain
                   │
                   └──────▶ scylla-proto

scylla-testkit (dev-dependency only) ──▶ scylla-core, scylla-domain, scylla-proto
```

The arrows go in one direction only. `scylla-core` defines its extension points as traits in `scylla-extension`. An edition gives the implementations to the core in an `Edition` value. The core calls the traits and never names an implementation.

This has three results:

- This repository builds and runs alone. The Community edition is the core plus the no-op implementations of `scylla-extension`.
- The Enterprise edition depends on a tag of this repository. Nothing in this repository changes for it.
- The core never needs a feature flag. `scylla-core` contains no `#[cfg(feature = ...)]`, and no `quota`, `plan` or `tenant`. `scripts/check-structure.sh` checks this.

`scylla-domain` holds all the pure types of the core features. It uses only `uuid`, `time` and `thiserror`. In a cold build it compiles in parallel with sqlx and tonic, and an edit in a store or a handler never recompiles it. `scylla-proto` holds the generated code of `proto/`, so `protoc` runs only when a `.proto` file changes.

## The API for the editions

The Enterprise repository uses these parts of this repository. A change to one of them can stop the Enterprise build when it moves to the new tag. Tell the Enterprise team about each such change.

| Part | Items |
|---|---|
| `scylla-extension` | `ResourceGuard`, `CapabilityProvider`, `ServiceProvider`, `CreateContext`, `DeleteContext`, `GuardError`, `Rejection`, `async_trait` |
| `scylla-core` | `app::Edition`, `server::run`, `db::Tx`, `db::migrate`, `convert`, `Error`, `features::users` |
| `scylla-domain` | `ResourceKind`, `users::UserId`, `projects::KIND` |
| `scylla-testkit` | `TestApp` |
| Schema | the tables `users` and `projects`, and the core migration ledger `_sqlx_migrations_core` |

The Enterprise SQL reads the core tables. The sqlx `query!` macros check this SQL against the core schema at compile time. So a core change that breaks the Enterprise edition stops its build, not production.

## Where a type goes

A type goes in `scylla-domain` when it compiles with the dependencies of that crate only. A type stays in the feature folder only when it needs sqlx, tonic, tokio or a `scylla-extension` trait. You do not choose: the compiler stops a type that needs IO in the domain crate, and the rule sends every other type there.

Examples: `Project`, `NewProject`, `ProjectPage`, `ProjectName` and `projects::KIND` are pure, so they are in `scylla-domain/src/projects.rs`. `ProjectsGrpc` holds a `PgPool`, so it stays in its feature folder.

Two results of this rule:

- A feature has two places: its pure types in `scylla-domain/src/<name>.rs`, and its code in `scylla-core/src/features/<name>/`. The file name and the folder name are the same, so you find one from the other.
- A conversion from a domain type to a type of another crate (`pb::Project`, `Rejection`) is a free function, `to_pb` or `to_rejection`, in the file that uses it. The orphan rule of Rust does not permit `impl From` between two types of other crates.

## Tree

```
.
├── Cargo.toml                 workspace, [workspace.dependencies], dev profile
├── .cargo/config.toml         linker for each target
├── .env                       DATABASE_URL for the sqlx macros and sqlx-cli
├── .sqlx/                     offline query data (just prepare)
├── buf.yaml                   buf lint and buf breaking configuration
├── docker-compose.yml         Postgres 18 on 127.0.0.1:55432
├── justfile                   all commands
├── AGENT.md                   rules for coding agents, and the list of skills
├── .claude/skills/            one procedure for each kind of task (see "Agent skills")
├── scripts/measure-build.sh   compile time measurements
├── scripts/check-structure.sh the structure rules as checks, run by just lint
├── proto/                     the only proto include root
│   └── scylla/
│       ├── capabilities/v1/capability_service.proto
│       └── projects/v1/{project.proto, project_service.proto}
├── scylla-domain/             the pure types of the core features, no IO
│   └── src/
│       ├── lib.rs
│       ├── error.rs           DomainError (validation)
│       ├── resource.rs        ResourceKind
│       ├── users.rs           UserId
│       └── projects.rs        KIND, ProjectId, ProjectName, Project, NewProject, ListProjects, ProjectPage
├── scylla-extension/          the contract between the core and the editions
│   └── src/
│       ├── lib.rs             private modules, one import path: scylla_extension::X
│       ├── error.rs           GuardError, Rejection
│       ├── context.rs         CreateContext, DeleteContext
│       ├── guard.rs           trait ResourceGuard + AllowAll
│       ├── capabilities.rs    trait CapabilityProvider + CommunityCapabilities
│       └── services.rs        trait ServiceProvider + NoExtraServices
├── scylla-proto/              the only codegen crate (build.rs + include_proto!)
├── scylla-core/
│   ├── sqlx.toml              migration ledger: _sqlx_migrations_core
│   ├── migrations/            users, projects
│   └── src/
│       ├── lib.rs
│       ├── app.rs             Edition, routes()
│       ├── server.rs          run(): env, tracing, pool, migrations, serve()
│       ├── error.rs           the only error type of the crate + mapping to tonic::Status
│       ├── convert.rs         uuid and timestamp conversions at the proto boundary
│       ├── db/{mod.rs, tx.rs, migrate.rs}
│       └── features/
│           ├── projects/{mod.rs, store.rs, service.rs, grpc.rs, tests.rs}
│           ├── capabilities/{mod.rs, grpc.rs}
│           └── users/{mod.rs, store.rs}
├── scylla-testkit/            starts the real gRPC server on a free port, gives clients and the channel
└── scylla-ce/
    ├── src/{lib.rs, app.rs, main.rs}   app.rs: AllowAll + CommunityCapabilities + NoExtraServices
    └── tests/api.rs
```

## Rules

Each file name has one meaning in all crates and in both repositories:

| File | Contents |
|---|---|
| `<domain crate>/src/<feature>.rs` | all the pure types of the feature: ids, validated values, entities, inputs, pages, `KIND`, capability name |
| `store.rs` | SQL with `query!`, free functions on `&mut Tx<'_>`; no tonic |
| `service.rs` | business logic: begins the transaction, calls the guard, calls the store, commits |
| `grpc.rs` | tonic server: `server()` constructor, request parsing, `to_pb` conversions |
| `extension.rs` | the implementation of a `scylla-extension` trait |
| `tests.rs` | unit tests of the feature |

- The call order is always `grpc -> service -> guard -> store`. A `grpc.rs` never calls `store.rs`. A `store.rs` never knows tonic.
- A `mod.rs` or `lib.rs` contains only `mod` and `pub use`. The public API of a feature is what its `mod.rs` re-exports.
- A feature folder has no `model.rs`. Its pure types are in the domain crate (see "Where a type goes"). Import them from the domain crate. `scylla-core` does not re-export them, so each type has one import path.
- No file has more than 300 lines. If a feature gets bigger, split it.
- Each crate has one error type, in `error.rs`.
- A feature has only the files that it needs.
- The guard can refuse, and can write to its own tables. It cannot change the created entity (the context does not contain it). It cannot commit: `commit()` takes the transaction by value, and the guard has only `&mut`.
- The code and the build files of this repository never refer to the Enterprise repository: no path, no crate, no file.

## Add a feature

Example: a `pipelines` feature in the core. Do the steps in this order. The skill `.claude/skills/add-core-feature` gives the same steps, with the details and the decisions that are already made.

1. `proto/scylla/pipelines/v1/pipeline.proto` (the entity) and `pipeline_service.proto` (the service, one `XxxRequest` and `XxxResponse` for each RPC). Add the two files to `scylla-proto/build.rs` and a `pub mod pipelines { pub mod v1 { ... } }` to `scylla-proto/src/lib.rs`. Run `buf lint`.
2. `scylla-domain/src/pipelines.rs`: all the pure types. `KIND`, `PipelineId`, the validated value types, the entity, the inputs, the page. Add the error variants to `scylla-domain/src/error.rs` and `pub mod pipelines;` to `lib.rs`.
3. `sqlx migrate add --sequential --simple --source scylla-core/migrations pipelines`, then write the SQL. Run `just migrate`: the `query!` macros of the next step need the table.
4. `scylla-core/src/features/pipelines/mod.rs` with `mod store;`, and `pub(crate) mod pipelines;` in `features/mod.rs`. Then `store.rs`: the SQL. Add each `mod` line to `mod.rs` when you create its file.
5. `service.rs`: begin, guard, store, commit.
6. `grpc.rs`: `server()`, the trait impl that calls `impl` methods that return `crate::Error`, and `fn to_pb(pipeline: Pipeline) -> pb::Pipeline`. Add `pub(crate) use grpc::server;` to `mod.rs`.
7. `tests.rs`: unit tests with `#[sqlx::test(migrator = "crate::db::MIGRATOR")]`.
8. `app.rs`: `.add_service(pipelines::server(...))` in `routes()`.
9. One helper for each RPC in `scylla-testkit`, and the integration tests `creates_lists_and_deletes_pipelines` and `rejects_invalid_pipeline_requests` in `scylla-ce/tests/api.rs`.
10. `just prepare`, `just lint`, `just test`.

An Enterprise feature is in the Enterprise repository. Its README gives the steps.

## Agent skills

`.claude/skills/` has one skill for each kind of task: `add-core-feature`, `add-rpc`, `add-field`, `change-extension-point`, `add-dependency`. Each skill gives the steps in order, the decisions that are already made, and the reference file to copy. Three skills hold the rules that the others load: `edit-proto`, `edit-migrations`, `write-tests`. Each change ends with `finish-change`. `AGENT.md` lists them.

`just lint` runs `scripts/check-structure.sh`. The script checks the rules that a command can check: the EE words, the file length, the `mod.rs` contents, the call direction, the dependency rules, the comments and the em dashes. A rule in a skill that a command can check goes in this script too.

## Extension points

- `ResourceGuard`: `before_create` and `after_delete` run in the transaction of the core service. A refusal is a `Rejection { code, message, details }`. The core sends it as `FAILED_PRECONDITION` with a `google.rpc.ErrorInfo` detail: `reason` is the code, `domain` is `scylla`, and `metadata` holds the details (for example `used` and `limit`).
- `CapabilityProvider`: the edition name and the list of optional features. `GetCapabilities` returns them, so one front end can show or hide the optional features. The core features are always present and are not in the list.
- `ServiceProvider`: the gRPC services that an edition adds next to the core services. The core gives it the `RoutesBuilder` and the pool after it adds its own services. The Community edition gives `NoExtraServices`. The request code of an edition service returns `scylla_core::Error`, so all editions map errors to gRPC codes in the same way.
- `Edition { guard, capabilities, services, migrations }`: everything that is different between the editions. `db::migrate` always applies the core migrations first, then the migrations of the edition.

## Run

Prerequisites: Docker, Rust 1.94 or later, `just`, `protoc`, and sqlx-cli with `sqlx.toml` support:

```sh
cargo install sqlx-cli --no-default-features --features postgres,sqlx-toml
```

`grpcurl` and `buf` are optional.

```sh
just db-up                              # Postgres on 127.0.0.1:55432
just migrate                            # core migrations
just run-ce                             # Community edition on 127.0.0.1:3000
just user alice                         # inserts an owner and prints its id
```

`BIND_ADDR` changes the address and `RUST_LOG` changes the log filter. Calls with grpcurl, from the repository root:

```sh
OWNER=$(just user alice)
g() { grpcurl -plaintext -import-path proto \
  -proto scylla/projects/v1/project_service.proto \
  -proto scylla/capabilities/v1/capability_service.proto "$@"; }

g localhost:3000 scylla.capabilities.v1.CapabilityService/GetCapabilities
g -d "{\"owner_id\":\"$OWNER\",\"name\":\"alpha\"}" localhost:3000 scylla.projects.v1.ProjectService/CreateProject
g -d "{\"owner_id\":\"$OWNER\",\"page_size\":2}" localhost:3000 scylla.projects.v1.ProjectService/ListProjects
g -d '{"project_id":"<id>"}' localhost:3000 scylla.projects.v1.ProjectService/DeleteProject
```

## Tests

```sh
just test                 # all tests
```

All tests use a real Postgres. `sqlx::test` creates one database for each test. The integration tests start the real gRPC server on a free port and call it with the generated tonic clients.

| Test | Where |
|---|---|
| CE creates 20 projects; pagination; invalid requests; community capabilities | `scylla-ce/tests/api.rs` |
| A rejection rolls back the guard writes; `after_delete` gets kind and owner; pages | `scylla-core/src/features/projects/tests.rs` |

## Compile times

Machine: Apple M5 Pro, 15 cores, 24 GB, macOS 26.5. Rust 1.98.1. `cargo build --workspace`, dev profile, sqlx macros connected to the database (not offline). To measure again: `just measure`.

These values come from before the split, when the EE crates were in the same workspace. Measure again before you compare.

| Change | ld-prime (Xcode linker) | lld 23 |
|---|---|---|
| Cold build (empty target folder) | 22.5 s | 23.5 s |
| One line in `scylla-domain` (`ProjectName::MAX_LEN`) | 1.3 s | 1.4 s |
| One line of SQL in `projects/store.rs` | 1.2 s | 1.4 s |
| One line in `projects/grpc.rs` (a field name in `list`) | 1.1 s | 1.2 s |

Each value is one run. The cold build of the same code changed between 29.4 s and 24.0 s on two earlier runs, so a difference of 5 s is noise of the machine. The differences between the two linkers are smaller than this noise, so on macOS `.cargo/config.toml` keeps ld-prime. A change in `scylla-domain` recompiles every crate above it, and it is still the slowest rebuild at 1.3 s.

Notes:

- Run cargo with `--workspace`, or on `scylla-ce`. These commands share one artifact for each crate. `cargo build -p scylla-domain` or `-p scylla-extension` calculates the features for that crate only (tokio loses `macros` and `rt-multi-thread`), and Cargo builds a second copy of tokio and sqlx. The fix, `resolver.feature-unification = "workspace"`, is still unstable in Cargo 1.98.
- Each dependency feature is set one time, in `[workspace.dependencies]`. No crate adds features, so `cargo build` and `cargo test` use the same feature set.
- Two costs cannot be removed: sqlx-macros uses syn 2 while the other macros use syn 3, so syn compiles two times; and sqlx-core turns on `tracing-attributes`.
- `[profile.dev.package.sqlx-macros] opt-level = 3` makes the `query!` checks faster.
- CI can build without a database: `SQLX_OFFLINE=true cargo build`. Run `just prepare` after each SQL change.

## Releases for the Enterprise edition

The Enterprise repository takes the crates of this repository from one Git tag:

```toml
[workspace.dependencies]
scylla-core = { git = "https://example.com/scylla.git", tag = "v0.1.0" }
scylla-domain = { git = "https://example.com/scylla.git", tag = "v0.1.0" }
scylla-extension = { git = "https://example.com/scylla.git", tag = "v0.1.0" }
scylla-testkit = { git = "https://example.com/scylla.git", tag = "v0.1.0" }
```

- All crates must come from the same tag, so that the types are the same.
- Cargo checks out the full repository for a Git dependency. So the `../proto` path of `scylla-proto/build.rs` works there too.
- Keep `.sqlx/` in the repository. An offline build of the Enterprise edition compiles `scylla-core` with the `.sqlx/` folder of this repository.
- The core ledger table (`_sqlx_migrations_core`) is separate from the Enterprise ledger table. So the two repositories add migrations without conflict.

## Known limits

- No authentication, as the brief asks. The owner id is in the request.
- No TLS on sqlx or tonic.
- The server has no graceful shutdown.
- `buf breaking` is configured, but it needs a Git history to compare against.
