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

- **Decision:** one table `actors (id, kind, owner_id, created_at)`. `ActorKind` is open. The core defines the kinds of its features (`user`, `agent`), and an edition adds its own. An actor can have an owner actor: the user of an agent, later the organization of a team. A feature that needs details for a kind adds its own table with `id REFERENCES actors (id)`.
- **Why:** a person, a bot or a later kind can act and own resources, and each reference keeps a real foreign key. The owner of an actor makes a tree of ownership (see 29).
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
- **Why:** the check is offline (no call to a token server). A holder can attenuate a token offline (`check if operation("scylla.projects.v1.ProjectService/ListProjects")`) before it gives it to a step of a pipeline. Only the first block can name the actor, so a later block cannot change it.
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

- **Decision:** `scylla-macros` (the `authz!` procedural macro), `scylla-domain` (pure types), `scylla-extension` (the contract), `scylla-proto` (generated code), `scylla-core`, `scylla-testkit`, `scylla-ce`.
- **Why:** the Enterprise repository needs the domain types and the contract without the core internals. Pure types compile in parallel with sqlx and tonic. `protoc` runs only on a proto change. `scylla-proto` gives its proto folder to the Enterprise build script with `links` metadata, so a Git dependency works. A procedural macro must be in its own crate, and `scylla-domain` gives it as `scylla_domain::authz!`.

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

- **Decision:** one `//!` line per file and a short `///` on each item, checked by the compiler. A long function body has steps: a blank line between two steps, and one `// Step.` line at the head of each step. No reasons in the code: they go here.
- **Why:** each item explains itself in the IDE and in `cargo doc`. A long function reads as a list of steps, and the code of a step tells how. The code stays short.
- **Not kept:** no comment in a body (a long function was hard to read); a comment on each line (noise that gets old).

## 18. Offline SQL data

- **Decision:** `.sqlx/` is in the repository.
- **Why:** CI builds with no database (`SQLX_OFFLINE=true`), and an offline Enterprise build compiles the core with this folder.

## 19. SpiceDB permissions

- **Decision:** SpiceDB holds the grants, in Postgres (its own database). The subject is always `actor`, so the actor kinds stay open. `actor#act_for = delegate`: the delegates of an actor act for it. A project has the roles `admin`, `editor` and `viewer` (`role + role->act_for`), with the groups `delete`, `edit` and `view`, and each route has its own permission (see 28). Each plugin registers the model of its types (`EditionBuilder::authz`, see 25), and `migrate` writes the full schema.
- **Why:** roles, sharing and delegation are relations, not code. An edition adds its definitions with no change to the core. The owners are not in SpiceDB (see 29).
- **Not kept:** permission checks in SQL in each feature.

## 20. Consistency between Postgres and SpiceDB

- **Decision:** no write goes to the two stores. A creation or a delete changes Postgres only. A grant changes SpiceDB only. After a delete, the server removes the grants on the object, and logs a failure. Each check that reaches SpiceDB is fully consistent.
- **Why:** a commit can fail in more than one way, and its result can even be unknown. With one store for each write, no undo is necessary. A grant on a deleted object reaches nothing, because each route reads the object from Postgres first, and ids are never used again. SpiceDB can answer from a cache up to 5 seconds old: a check just after a grant would give a false refusal, so correctness comes first. A ZedToken can make a check faster later (`Check::at_least_as_fresh`).
- **Not kept:** the owner written to SpiceDB before the commit, and deleted again after a failed commit (an unknown commit result leaves an owner with no right); a transactional outbox (a worker, a refusal just after a create, and one write for each resource).

## 21. An unknown resource gives `PERMISSION_DENIED`

- **Decision:** a delete of an unknown project gives the same answer as a delete without the permission. The route reads the owner first: no owner gives the refusal, with the same message.
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


## 25. A typed permission model

- **Decision:** the procedural macro `authz!` declares each SpiceDB definition next to its Rust type, in the SpiceDB schema language. It implements `Resource` and `Model` for the type, and makes one constant for each name: `Project::DELETE` is a `Permission<Project>`, `Project::VIEWER` is a `Relation<Project, Actor>`. `Permission::on(id)` and `Relation::of(object, subject)` take typed ids, and only they make a `Check` or a `Relationship`. `EditionBuilder::authz::<T>()` adds the part of a type, and the edition writes the schema from the parts. `authz! { extend Project as ProjectAudit { ... } }` adds relations and permissions to a definition of another plugin.
- **Why:** one source for the schema and the code. The compiler stops a wrong name, a wrong id type, and a wrong name in a permission expression (an arrow is checked on the subject type). SpiceDB checks the full schema each time a test writes it. An edition adds objects, relations and permissions with no change to the core.
- **Not kept:** schema text and names as text (no check before run time); a `macro_rules!` macro (no constants in upper case, no check of the expressions); a typed SpiceDB client from another crate (it replaces the official client, and no crate is a standard); an extension that changes an existing permission (an extension only adds, so the core rules stay true).

## 26. The operation is the gRPC method

- **Decision:** `OperationLayer` adds the gRPC method of each request (`scylla.projects.v1.ProjectService/CreateProject`) to the request. `Gate::caller` gives it to the token check as `operation`.
- **Why:** no name to write and keep in each handler, and the same name that the client sees. A token can allow one service with a prefix: `check if operation($op), $op.starts_with("scylla.projects.v1.ProjectService/")`.
- **Not kept:** a name in each handler (`projects.create`): a handler can forget it or write a wrong one.

## 27. One SpiceDB store for each test

- **Decision:** the tests use `spicedb serve-testing` (`spicedb-test`, port 55052). Each `TestApp` uses a new key, and serve-testing gives an empty store in memory to each key. `.cargo/config.toml` runs the tests on 8 threads.
- **Why:** the tests run in parallel with different schemas (an edition extends a definition) and with no shared relationships, as `sqlx::test` gives a new database to each test. The dev SpiceDB keeps only dev data. `sqlx::test` shares 20 database connections between the tests that run at the same time. With 16 threads, a test waited 30 seconds for a connection.
- **Not kept:** the dev SpiceDB for the tests: the schema write of one test changes the schema of all the tests, and the relationships stay after the tests.

## 28. One permission for each route

- **Decision:** each route that verifies a token checks its own permission: the RPC name in snake case, on the object of the route. `CreateProject` checks `create_project` on the owner. `DeleteProject` checks `delete_project` on the project and on its owners. A route permission uses a group: `act_for`, `delete`, `edit`, `view`. A feature adds the permissions of its routes on an owner with `extend Actor as <Feature>Owner`, and on the object only when one object can be shared. The gate refuses a check of a permission that is not the route permission. After the route, the server refuses a success when the route verified a token and passed no check of its route permission: it answers `PERMISSION_DENIED` and logs an error.
- **Why:** a role can give one route and not the others: a quota reader reads quotas and creates nothing. Groups keep the common case short, and a new role changes the schema, not the code. A route that forgets its check, or that checks a group, fails in the tests of its success.
- **Not kept:** checks of groups in the routes (a new role gives all the routes of the group, or a change of code); a list of route permissions in a table (a second source next to the schema); a stop before the route runs when a check is missing (the server does not know the object of the route before the route reads the request). A route that must also check the permission of another route needs a new decision.

## 29. Ownership in Postgres, grants in SpiceDB

- **Decision:** Postgres holds who owns what: `owner_id` of a resource, and `actors.owner_id` (the user of an agent). SpiceDB holds only the rights that a person gives on purpose: delegates, roles on an owner, roles on one object. The gate finds the rights of a request in this order: the caller is the owner of the object, or an actor above the owner, with no SpiceDB call; else one `CheckBulkPermissions` call asks for the route permission on the object and on each actor of the owner chain. A grant on an actor goes down to all that it owns. A check of an object that is not an actor names the owner: `Project::DELETE_PROJECT.on(id).owned_by(owner)`.
- **Why:** a creation or a delete is one Postgres transaction, with no write to SpiceDB and no undo. Most requests are by the owner and need no SpiceDB call. SpiceDB stores only the grants, not one relationship for each actor and resource. The owner is business data: lists, quotas under concurrency, foreign keys and hooks read it in SQL. A grant on one object stays possible, for precise sharing.
- **Not kept:** the owner in SpiceDB only (a SpiceDB write at each creation, no sorted lists, no quota lock, no foreign key); the owner in the two stores (two copies that must agree after each commit); the model of Kubernetes, with roles only on an owner (no sharing of one object).
