# Agent notes

This is the public repository: the core and the Community edition. `README.md` tells what it is and how to run it. `DECISIONS.md` tells why the code has its shape. This file tells how to write code here. Follow it in each change.

The Enterprise edition is in a separate repository (`../ee` in this POC). Do not change it from here.

## Skills

Load the skill of the task from `.claude/skills/`:

- `add-feature`: a new resource, an RPC, or a field.
- `change-extension-point`: a new hook, context value, authenticator need or builder method in `scylla-extension`.
- `finish-change`: the checklist at the end of each change.

## Structure

- A feature is one folder `scylla-core/src/features/<feature>/` with only the files that it needs:

  | File | Contents |
  |---|---|
  | `mod.rs` | `mod` and `pub use` lines only |
  | `plugin.rs` | the `Plugin` impl: it registers the services, hooks, authenticators and capability |
  | `grpc.rs` | the tonic handler: authenticate, parse, authorize, call the service, answer |
  | `service.rs` | free functions: begin the transaction, run the hooks, call the store, commit |
  | `store.rs` | the SQL, free functions on `&mut Tx<'_>`, no tonic |
  | `tests.rs` | unit tests of what the API cannot show |

- The pure types of a feature go in `scylla-domain/src/<feature>.rs`. No `model.rs`.
- The call order is `grpc -> service -> hooks -> store`. A `grpc.rs` never calls a store.
- Import the module, not the function: `store::insert(...)`.
- Each crate has one error type. The routes return `scylla_core::Error`.
- Keep each file below 300 lines.
- The code and the build files never refer to the Enterprise repository. Its feature names (`quota`, `plan`, `tenant`, `enterprise`) are not in the code.

## Ids, actors and access

- Each id is `Id<T>` (`ActorId`, `ProjectId`). The format is only in `scylla-domain/src/codec.rs`. Never use `Uuid` outside `to_db()` and `from_db()` in a store.
- Each resource references `actors (id)`: `owner_id` (who owns it) and `created_by` (who did it). The actor kinds are open: a feature defines its own `KIND`.
- Zero trust: each RPC that acts calls `gate.caller(&request)`, then `gate.authorize(&caller, <permission>.on(id))` for each permission that it needs. The token check uses the gRPC method of the request as the operation (`OperationLayer` adds it). Only `TokenService`, health and capabilities are public.
- The permission model is Rust code. `scylla_domain::authz!` declares each SpiceDB definition next to its type, in the SpiceDB schema language. It makes the constants: `Project::DELETE` is a `Permission<Project>`, `Project::OWNER` is a `Relation<Project, Actor>`. Never write the name of a definition, relation or permission as text.
- `Permission::on(id)` takes an id of its resource. `Relation::of(object, subject)` takes an id of each of its two resources. The compiler stops a wrong id, and a wrong name in a permission expression.
- An `owner_id` that the request does not set is the caller. The caller must have `Actor::ACT_FOR` on the owner.
- A write that gives rights ends with `access::commit(tx, gate, &grants)`: it writes the relationships, then commits, and deletes them again if the commit fails. A delete removes the relationships with `gate.delete_object(&Object::of(id))` after the commit.
- A plugin registers the model of its types with `EditionBuilder::authz::<Type>()`. An edition adds relations and permissions to a core definition with `authz! { extend <Type> as <Holder> { ... } }`, registered after the core.
- A secret never goes to the logs or the database in clear. The server stores public keys and hashes only.

## Comments

This project overrides the global "no comments" rule.

- Each `.rs` file starts with one `//!` line that tells what the file is.
- Each item, private ones too, has a `///` of one or two short sentences: what it is or does. The compiler stops a missing doc (`missing_docs`, `clippy::missing_docs_in_private_items`).
- No `# Why` sections and no long argument in the code. A decision and its reason go in `DECISIONS.md`.
- No `//` in a body, except `// given`, `// when`, `// then` in a test.
- Write in ASD-STE100 Simplified Technical English. No em dashes.

## Tests

- First, black-box gRPC tests: one binary per app (`tests/api/main.rs`), one module per feature.
- A unit test only for what the API cannot show (rollback, hook order) and for pure functions.
- Name: `<rpc>_<expected behavior>`. A `///` line tells the rule under test.
- Body: `// given`, `// when`, `// then` blocks.
- One helper per core RPC in `scylla-testkit`, sent as an actor with `authed`. `app.grant(&[...])` writes relationships, and `app.can(actor, check)` checks a permission.
- Each test makes its own data. `sqlx::test` gives each test a new database, and `TestApp` gives each test a new SpiceDB store.

## Protos

- All ids use `scylla.common.v1.Id`. Lists use `PageRequest` and `PageResponse`.
- Each service, RPC, message and field has a comment (`buf lint`, category `COMMENTS`).
- Each RPC has its own request and response messages.
- Never reuse or change a field number. A breaking change needs a `v2` package: ask the user.

## SQL

- Never change a migration that ran on a database. Write a new one.
- An id column is `uuid`. A reference to an actor is `REFERENCES actors (id)`, with an index.
- After a SQL change, run `just prepare` and keep `.sqlx/` in the change.

## Dependencies

- Use the standard crate for a solved problem (`clap` for the command line). Do not write it by hand.
- Each dependency is in `[workspace.dependencies]` with all its features. A crate writes only `<name>.workspace = true`.
- Tell the user about each change of version or feature: the Enterprise repository uses the same lines.

## Commands

- `just run`, `just test`, `just lint`, `just prepare`, `just schema`, `just keycloak`, `just actor <kind>`, `just token <actor id>`.
- Run cargo with `--workspace` or `-p scylla-ce`. On another crate, Cargo builds tokio and sqlx again.

## Pitfalls

- The database is on port 55432. SpiceDB is on 55051, the SpiceDB of the tests on 55052, and Keycloak on 58080. Port 5432 and the container `scylla-postgres` belong to another project. Do not touch them.
- Use the latest tools: `cargo install sqlx-cli --locked --features sqlx-toml`, and `npm install -g @bufbuild/buf`. Remove an old version before you install a new one.
- A clap `Args` struct needs a unique `#[group(id = ...)]`. The test `cli_is_valid` finds a conflict.
