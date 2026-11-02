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

- **Decision:** one table `actors (id, kind, created_at)`. `ActorKind` is open. The core defines the kinds of its features (`user`, `agent`), and an edition adds its own. Each actor has an `identity` relation in SpiceDB. A feature that needs details for a kind adds its own table with `id REFERENCES actors (id)`.
- **Why:** a person, a bot or a later kind can act and own resources, and each reference keeps a real foreign key.
- **Not kept:** one table for each kind with no common identity (no common foreign key); a pair `(kind, id)` with no foreign key (no integrity).

## 5. Owner and author

- **Decision:** a resource has `owner_id` (who owns it) and `created_by` (who did it). A request that names no owner creates for the caller.
- **Why:** an actor can act for another actor. Limits count by owner, an audit reads the author.

## 6. Zero trust

- **Decision:** each request carries a short-lived token, and the server checks it on each request: signature, expiry, revocation. Then SpiceDB checks the exact permission. The right of a request is the SpiceDB permission AND the checks of the token. A long-lived credential is used only to get a token, never on a request.
- **Why:** no request is trusted because of where it comes from. A stolen token is valid for minutes, and it can carry fewer rights than its actor.
- **Not kept:** an actor in a request field or a trusted header (any client can claim any actor); long-lived API keys on each request.

## 7. Biscuit tokens

- **Decision:** the tokens are Biscuits, signed with an Ed25519 root key (`SCYLLA_TOKEN_PRIVATE_KEY`). The first block holds `actor(id, kind)`, `expires(unix)` and a time check. The default lifetime is 10 minutes. The key id is in each token, and `SCYLLA_TOKEN_PUBLIC_KEYS` keeps older public keys valid after a rotation. `token revoke` revokes a token and each token made from it.
- **Why:** the check is offline (no call to a token server). A holder can attenuate a token offline (`check if operation("projects.list")`) before it gives it to a step of a pipeline. Only the first block can name the actor, so a later block cannot change it.
- **Not kept:** JWT (no offline attenuation); opaque tokens in a table (a database read for each request). Biscuit 6.0.0 needs its `datalog-macro` feature to compile, and that feature brings `proc-macro-error2`, which gives a future-compatibility warning from upstream.

## 8. How each actor gets a token

- **Decision:** `TokenService` has two exchanges. An agent signs a short assertion (agent id, key id, audience `scylla`, expiry of at most 2 minutes, nonce) with its Ed25519 private key: `ExchangeAgentAssertion`. A person gives an OIDC ID token: `ExchangeIdToken`, with issuer, audience, expiry and a JWKS signature checked (no HMAC). The first sign-in creates the user. The CLI `token create` signs a token for a first admin.
- **Why:** an agent keeps its private key, and the server keeps only the public key: nothing secret is stored. The nonce table stops a replay. The persons use the identity provider of the company, with its SSO and MFA.
- **Not kept:** a shared secret for agents; a password store in Scylla.

## 9. Opaque ids

- **Decision:** `Id<T>` holds 128 bits, stored in a Postgres `uuid` column. Only `codec.rs` knows the text form and how to make a new id (UUIDv7 today).
- **Why:** a change to ULID or another 128-bit format changes one file. The type parameter stops a project id where an actor id goes. UUIDv7 ids sort in creation order, so pages use the id.
- **Not kept:** a format per entity (two text forms in one API); text ids in the database.

## 10. One proto id message

- **Decision:** all ids use `scylla.common.v1.Id { string value }`. The field name tells which entity. One generic `FromProto` and `ToProto` impl serves each `Id<T>`.
- **Why:** one parse and one error for all ids, no code for a new id kind. Typed id messages give no safety to a TypeScript client (same shape, same type), and the Rust domain keeps the typed ids.
- **Not kept:** one message per id kind; a nested `Uuid` message inside each id (two levels for clients).

## 11. Keyset pagination

- **Decision:** `PageRequest { size, token }` and `PageResponse { next_token }` in `scylla.common.v1`. The token is the last id. The store reads one row more than the page size.
- **Why:** a page stays correct when rows change, and its cost does not grow with its position.

## 12. Errors

- **Decision:** one `scylla_core::Error` for the core and the edition routes. A bad field gives `INVALID_ARGUMENT` with a `google.rpc.BadRequest` that names the field. A refusal gives `FAILED_PRECONDITION` with a `google.rpc.ErrorInfo` (`reason`, `domain = scylla`, numbers in `metadata`). Other errors give `INTERNAL` and go to the log.
- **Why:** a client reads codes and details, not messages, and each edition maps errors in the same way.

## 13. Crates

- **Decision:** `scylla-domain` (pure types), `scylla-extension` (the contract), `scylla-proto` (generated code), `scylla-core`, `scylla-testkit`, `scylla-ce`.
- **Why:** the Enterprise repository needs the domain types and the contract without the core internals. Pure types compile in parallel with sqlx and tonic. `protoc` runs only on a proto change. `scylla-proto` gives its proto folder to the Enterprise build script with `links` metadata, so a Git dependency works.

## 14. Command line and settings

- **Decision:** `clap` (derive and `env`). Each setting is a flag and an environment variable, with a default. Commands: `serve` (the default) and `migrate`.
- **Why:** a standard tool gives help, checks and errors for free. A deploy runs `migrate` as one job before the servers, so servers never race to migrate. `serve` checks at start that each migration is applied.

## 15. Server for production

- **Decision:** graceful shutdown on SIGTERM and Ctrl+C, the standard `grpc.health.v1` service (it gives `NOT_SERVING` when the stop starts), one log line per request, a request timeout, pool limits and a Postgres `statement_timeout`. The request timeout is longer than the statement timeout.
- **Why:** Kubernetes and Docker stop a pod with SIGTERM and probe its health. A slow statement fails in Postgres first and frees its locks. A client error is an `INFO` response, not a server failure.

## 16. Tests

- **Decision:** black-box gRPC tests first, one test binary per app with one module per feature, `// given`, `// when`, `// then`. Unit tests only for what the API cannot show.
- **Why:** the API is the contract. One binary links one time. The same server code runs in tests and in production.

## 17. Comments

- **Decision:** one `//!` line per file and a short `///` on each item, checked by the compiler. No reasons in the code: they go here.
- **Why:** each item explains itself in the IDE and in `cargo doc`, and the code stays short.

## 18. Offline SQL data

- **Decision:** `.sqlx/` is in the repository.
- **Why:** CI builds with no database (`SQLX_OFFLINE=true`), and an offline Enterprise build compiles the core with this folder.

## 19. SpiceDB permissions

- **Decision:** SpiceDB holds the permissions, in Postgres (its own database). The subject is always `actor`, so the actor kinds stay open. `actor#act_for = identity + delegate`: an actor acts for itself and for the actors that delegate to it. A project gives `delete`, `edit` and `view` through `owner->act_for`. Each plugin registers its piece of schema (`EditionBuilder::authz_schema`), and `migrate` writes the full schema.
- **Why:** roles, sharing and delegation are relations, not code. An edition adds its definitions with no change to the core. A registered agent is a delegate of its user, so the user acts for the agent, not the reverse.
- **Not kept:** permission checks in SQL in each feature.

## 20. Consistency between Postgres and SpiceDB

- **Decision:** a write sends its relations to SpiceDB before the Postgres commit. A SpiceDB error rolls back the transaction. A failed commit deletes the relations again (`access::commit`). A delete removes the relations after the commit, and logs a failure. Each check is fully consistent.
- **Why:** no worker to run, and no relation can give a right to a row that was not written. SpiceDB can answer from a cache up to 5 seconds old: a check just after a write would give a false refusal, so correctness comes first. A ZedToken can make a check faster later (`Check::at_least_as_fresh`).
- **Not kept:** a transactional outbox (a worker, and a refusal just after a create).

## 21. An unknown resource gives `PERMISSION_DENIED`

- **Decision:** a delete of an unknown project gives the same answer as a delete without the permission.
- **Why:** a caller does not learn which ids exist.

## 22. gRPC-Web

- **Decision:** the server accepts HTTP/1.1 and gRPC-Web (`tonic-web`) on the same port as gRPC. CORS allows only the origins of `SCYLLA_CORS_ORIGINS`, with the gRPC headers exposed.
- **Why:** a browser calls the API with no proxy. An empty list allows no web origin.

## 23. TLS in the server

- **Decision:** `SCYLLA_TLS_CERT` and `SCYLLA_TLS_KEY` turn on TLS (rustls with ring). Clap requires the two together. The process installs ring as its only crypto provider (`crypto::install`), for tonic and for the HTTP client of OIDC.
- **Why:** a deploy with no proxy can still encrypt. ring builds with no extra tool on Windows, Linux and macOS.

## 24. Tools

- **Decision:** the latest version of each tool and crate. The recipes call `cargo sqlx` and `buf`.
- **Why:** one known version for each tool, with the latest fixes. The README gives the install lines.

