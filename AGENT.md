# Agent notes

This is the public repository: the core and the Community edition. Read `README.md` first. It gives the dependency direction, the file rules and the recipe to add a feature.

The Enterprise edition is in a separate repository (`../ee` in this POC). Do not change it from a task in this repository.

## Skills

For a task, load its skill from `.claude/skills/`. A skill gives the steps and the decisions that are already made.

- `add-core-feature`: a new resource with its table and gRPC service in the core.
- `add-rpc`: a new operation on an existing core feature.
- `add-field`: a new field on an existing entity, from the migration to the tests.
- `change-extension-point`: a new hook, context value or trait in `scylla-extension`.
- `add-dependency`: a new crate, or a change of version or features.
- `edit-proto`, `edit-migrations`, `write-tests`: the rules for protos, SQL and tests. The other skills load them.
- `finish-change`: the checklist before you report a change as done. Do it after each change.

For an Enterprise feature, work in the Enterprise repository. It has its own skills.

## Rules to keep

- Do not add EE words (`quota`, `plan`, `tenant`, `enterprise`) or `#[cfg(feature = ...)]` to `scylla-core`. If the core needs to know about an edition, add or change a trait in `scylla-extension`.
- The code and the build files of this repository never refer to the Enterprise repository: no path, no crate, no file.
- A change to the API for the editions (`README.md`, section "The API for the editions") can stop the Enterprise build. Tell the user about each such change.
- Put each pure type of a feature in `scylla-domain/src/<feature>.rs`. A type is pure when it compiles with the dependencies of that crate only. Do not add a `model.rs` to a feature folder.
- Do not add sqlx, tonic or tokio to `scylla-domain`. If a type needs one of them, keep it in the feature folder.
- Import a domain type from `scylla-domain`. Do not re-export it from `scylla-core`.
- To convert a domain type into a type of another crate, write a free function (`to_pb`, `to_rejection`). The orphan rule does not permit `impl From` between two types of other crates.
- Import the module, not the function: `use super::store;`, then `store::insert(...)`. Store and service are free functions, not structs, because the transaction is a parameter of each call.
- Keep the call order `grpc -> service -> guard -> store`.
- The POC has no authentication. Do not add an access check. If a request names a role, write `No access check.` in the RPC comment, and tell the user.
- Keep `mod.rs` and `lib.rs` to `mod` and `pub use` lines only.
- Keep each file below 300 lines.
- Put each new dependency in `[workspace.dependencies]` with all its features. Do not add features in a crate `Cargo.toml`.
- Do not use em dashes in code, comments or documentation.

## Commands

- Run cargo with `--workspace`, or with `-p scylla-ce`. With `-p` on another crate, Cargo builds a second copy of tokio and sqlx.
- After a change to SQL, run `just prepare`. It refreshes `.sqlx` for all targets, tests included. Keep `.sqlx` in the repository: the offline build of the Enterprise edition uses it.
- Before you report a change as done, do the `finish-change` skill. `just lint` also runs `scripts/check-structure.sh`, which checks the structure rules of this file.

## Pitfalls

- The database of this POC is on port 55432. Port 5432 and the container `scylla-postgres` belong to another project of the user. Do not touch them.
- The Enterprise repository uses the same database and the same `DATABASE_URL`. Its migrations can be in the database. This has no effect on the core.
- sqlx-cli must have the `sqlx-toml` feature. Without it, `just migrate` stops with "the `sqlx-toml` feature was not enabled".
- Each repository with migrations has its own ledger table in `sqlx.toml`. Do not change `table-name` on a database that has data.
- The `sqlx::test` pool has 5 connections. A test that needs more connections takes `(PgPoolOptions, PgConnectOptions)`, builds its own pool, and closes it at the end.
