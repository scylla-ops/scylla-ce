# Decisions

Each entry gives the decision, the reason, and the alternatives that we did not keep. Add an entry when a change makes a new decision. Change an entry when a decision changes.

## 1. Two repositories, one direction

- **Decision:** the public repository (core and Community edition) never refers to the private repository (Enterprise edition). The private repository uses the public crates from one Git tag.
- **Why:** the public repository builds and runs alone, and the Enterprise features need no change in the core.
- **Not kept:** cargo feature flags in one repository. They put Enterprise code in the public code.

## 2. Plugins and an edition builder

- **Decision:** a feature, core or Enterprise, is one `Plugin`. It registers its capability, migrations, authenticators, hooks and gRPC services on an `EditionBuilder`. `Edition` has private fields.
- **Why:** one place for each feature, and one recipe for core and Enterprise. A new extension point is a new builder method, so no edition breaks.
- **Not kept:** a struct with one public field for each trait (it broke each edition at each new field, and allowed one guard only); a generic event bus with a type map (harder to read).

## 3. Hooks by event and by resource kind

- **Decision:** one small trait for each event (`BeforeCreate`, `AfterDelete`). A hook is registered for one resource kind. The hooks run in registration order, in the transaction of the core write. The first error stops the run.
- **Why:** a hook implements only what it needs, never filters the kind, and a refusal rolls back the core write and all hook writes together.
- **Not kept:** one trait with all events; hooks after the commit (no rollback).

## 4. Actors

- **Decision:** one table `actors (id, kind, created_at)`. `ActorKind` is open: the core defines no kind. A feature that needs details for a kind adds its own table with `id REFERENCES actors (id)`.
- **Why:** a person, a bot or a later kind can act and own resources, and each reference keeps a real foreign key.
- **Not kept:** a `users` table (it closes the model to persons); a pair `(kind, id)` with no foreign key (no integrity).

## 5. Owner and author

- **Decision:** a resource has `owner_id` (who owns it) and `created_by` (who did it). A request that names no owner creates for the caller.
- **Why:** an actor can act for another actor. Limits count by owner, an audit reads the author.

## 6. Authentication chain

- **Decision:** the `Authenticator` plugins find the actor of each request, in order. No credential gives `UNAUTHENTICATED`. The dev authenticator `TrustedHeader` reads `x-scylla-actor`. It is off by default: `SCYLLA_TRUST_ACTOR_HEADER=true` turns it on, and the server warns at start.
- **Why:** real authentication (tokens for persons, keys for bots) can come later as plugins, with no change to the features. A dev shortcut must not reach production by error.
- **Not kept:** the actor as a request field (any client can claim any actor).

## 7. Opaque ids

- **Decision:** `Id<T>` holds 128 bits, stored in a Postgres `uuid` column. Only `codec.rs` knows the text form and how to make a new id (UUIDv7 today).
- **Why:** a change to ULID or another 128-bit format changes one file. The type parameter stops a project id where an actor id goes. UUIDv7 ids sort in creation order, so pages use the id.
- **Not kept:** a format per entity (two text forms in one API); text ids in the database.

## 8. One proto id message

- **Decision:** all ids use `scylla.common.v1.Id { string value }`. The field name tells which entity. One generic `FromProto` and `ToProto` impl serves each `Id<T>`.
- **Why:** one parse and one error for all ids, no code for a new id kind. Typed id messages give no safety to a TypeScript client (same shape, same type), and the Rust domain keeps the typed ids.
- **Not kept:** one message per id kind; a nested `Uuid` message inside each id (two levels for clients).

## 9. Keyset pagination

- **Decision:** `PageRequest { size, token }` and `PageResponse { next_token }` in `scylla.common.v1`. The token is the last id. The store reads one row more than the page size.
- **Why:** a page stays correct when rows change, and its cost does not grow with its position.

## 10. Errors

- **Decision:** one `scylla_core::Error` for the core and the edition routes. A bad field gives `INVALID_ARGUMENT` with a `google.rpc.BadRequest` that names the field. A refusal gives `FAILED_PRECONDITION` with a `google.rpc.ErrorInfo` (`reason`, `domain = scylla`, numbers in `metadata`). Other errors give `INTERNAL` and go to the log.
- **Why:** a client reads codes and details, not messages, and each edition maps errors in the same way.

## 11. Crates

- **Decision:** `scylla-domain` (pure types), `scylla-extension` (the contract), `scylla-proto` (generated code), `scylla-core`, `scylla-testkit`, `scylla-ce`.
- **Why:** the Enterprise repository needs the domain types and the contract without the core internals. Pure types compile in parallel with sqlx and tonic. `protoc` runs only on a proto change. `scylla-proto` gives its proto folder to the Enterprise build script with `links` metadata, so a Git dependency works.

## 12. Command line and settings

- **Decision:** `clap` (derive and `env`). Each setting is a flag and an environment variable, with a default. Commands: `serve` (the default) and `migrate`.
- **Why:** a standard tool gives help, checks and errors for free. A deploy runs `migrate` as one job before the servers, so servers never race to migrate. `serve` checks at start that each migration is applied.

## 13. Server for production

- **Decision:** graceful shutdown on SIGTERM and Ctrl+C, the standard `grpc.health.v1` service (it gives `NOT_SERVING` when the stop starts), one log line per request, a request timeout, pool limits and a Postgres `statement_timeout`. The request timeout is longer than the statement timeout.
- **Why:** Kubernetes and Docker stop a pod with SIGTERM and probe its health. A slow statement fails in Postgres first and frees its locks. A client error is an `INFO` response, not a server failure.

## 14. Tests

- **Decision:** black-box gRPC tests first, one test binary per app with one module per feature, `// given`, `// when`, `// then`. Unit tests only for what the API cannot show.
- **Why:** the API is the contract. One binary links one time. The same server code runs in tests and in production.

## 15. Comments

- **Decision:** one `//!` line per file and a short `///` on each item, checked by the compiler. No reasons in the code: they go here.
- **Why:** each item explains itself in the IDE and in `cargo doc`, and the code stays short.

## 16. Offline SQL data

- **Decision:** `.sqlx/` is in the repository.
- **Why:** CI builds with no database (`SQLX_OFFLINE=true`), and an offline Enterprise build compiles the core with this folder.

## 17. API keys

- **Decision:** the core plugin `ApiKeys` authenticates `authorization: Bearer scy_<64 hex>`. The table `api_keys` keeps only the SHA-256 hash, the actor, and `revoked_at`. The CLI makes and revokes keys (`key create`, `key revoke`), and makes actors (`actor create`).
- **Why:** each actor kind (a service, a bot, a person with a personal key) can authenticate now, with no external system. A key has 256 random bits, so a fast hash is safe, and a leaked database gives no key. The prefix lets a secret scanner find a leaked key. The CLI makes the first keys, so no RPC must exist before a key exists.
- **Not kept:** a slow password hash (no gain for random keys); a key stored in clear.

## 18. TLS in the server

- **Decision:** `SCYLLA_TLS_CERT` and `SCYLLA_TLS_KEY` turn on TLS (rustls with ring). Clap requires the two together.
- **Why:** a deploy with no proxy can still encrypt. ring builds with no extra tool on Windows, Linux and macOS.

## 19. Tools

- **Decision:** the latest version of each tool and crate. The recipes call `cargo sqlx` and `buf`.
- **Why:** one known version for each tool, with the latest fixes. The README gives the install lines.
