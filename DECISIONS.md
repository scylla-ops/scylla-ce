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

- **Decision:** one table `actors (id, kind, created_at)`. `ActorKind` is open. The core defines the kinds of its features (`user`, `agent`), and an edition adds its own. A feature that needs details for a kind adds its own table with `id REFERENCES actors (id)`: `users`, `agents`. An actor has no owner and no role on another actor: it acts only for itself.
- **Why:** a person, a bot or a later kind can act, and each reference keeps a real foreign key. The rights come from the workspaces (see 32), not from a tree of actors.
- **Not kept:** one table for each kind with no common identity (no common foreign key); a pair `(kind, id)` with no foreign key (no integrity); an owner of an actor and the role `delegate` (a second way to give rights, next to the roles in a workspace).

## 5. Workspace and author

- **Decision:** a resource has one container, its workspace (`workspace_id`), and an author (`created_by`). A creation names its workspace. Only a workspace has owners: the rows of `workspace_owners`, one user or more (see 38).
- **Why:** a resource belongs to its workspace, not to a person: it stays when its author leaves. Limits count by workspace, an audit reads the author.

## 6. Zero trust

- **Decision:** each request carries a short-lived token, and the server checks it on each request: signature, expiry, revocation. Then SpiceDB checks the exact permission. The right of a request is the SpiceDB permission AND the checks of the token. A long-lived credential is used only to get a token, never on a request.
- **Why:** no request is trusted because of where it comes from. A stolen token is valid for minutes, and it can carry fewer rights than its actor.
- **Not kept:** an actor in a request field or a trusted header (any client can claim any actor); long-lived API keys on each request.

## 7. Biscuit tokens

- **Decision:** the tokens are Biscuits, signed with an Ed25519 root key (`SCYLLA_TOKEN_PRIVATE_KEY`). The first block holds `actor(id, kind)`, `expires(unix)` and a time check. The default lifetime is 10 minutes. The key id is in each token, and `SCYLLA_TOKEN_PUBLIC_KEYS` keeps older public keys valid after a rotation. `token revoke` revokes a token and each token made from it.
- **Why:** the check is offline (no call to a token server). A holder can attenuate a token offline (`check if operation("scylla.projects.v1.ProjectService/ListProjects")`) before it gives it to a step of a pipeline. Only the first block can name the actor, so a later block cannot change it.
- **Not kept:** JWT (no offline attenuation); opaque tokens in a table (a database read for each request). Biscuit 6.0.0 needs its `datalog-macro` feature to compile, and that feature brings `proc-macro-error2`, which gives a future-compatibility warning from upstream.

## 8. How each actor gets a token

- **Decision:** an agent signs a short assertion (agent id, key id, audience `scylla`, expiry of at most 2 minutes, nonce) with its Ed25519 private key: `ExchangeAgentAssertion`. A person signs in with a password (`AccountService.SignIn`), with an identity provider in a browser (`/auth/<slug>/start`, see 35), or with the OIDC ID token of a native client (`ExchangeIdToken`, issuer, audience, expiry and a JWKS signature checked, no HMAC). Each sign-in of a person opens a session (see 34). The CLI `token create` signs a token for a first admin.
- **Why:** an agent keeps its private key, and the server keeps only the public key. The nonce table stops a replay. A person can use the identity provider of the company, or an account of Scylla (see 33).
- **Not kept:** a shared secret for agents.

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

- **Decision:** SpiceDB holds the roles, in Postgres (its own database). The subject is always `actor`, so the actor kinds stay open. A workspace has the roles `admin`, `member` and `viewer`, with the groups `manage`, `work` and `read`. A project has the roles `admin`, `editor` and `viewer`, with the groups `delete`, `edit` and `view`. Each route has its own permission (see 28). Each plugin registers the model of its types (`EditionBuilder::authz`, see 25), and `migrate` writes the full schema.
- **Why:** roles and sharing are relations, not code. An edition adds its definitions with no change to the core. The owners of the workspaces are not in SpiceDB (see 29).
- **Not kept:** permission checks in SQL in each feature.

## 20. Consistency between Postgres and SpiceDB

- **Decision:** no write goes to the two stores. A creation or a delete changes Postgres only. A grant changes SpiceDB only. After a delete, the server removes the roles on the object, and logs a failure. An accepted invitation writes its role first, then marks the invitation: a retry writes the same role again with no harm. Each check that reaches SpiceDB is fully consistent.
- **Why:** a commit can fail in more than one way, and its result can even be unknown. With one store for each write, no undo is necessary. A grant on a deleted object reaches nothing, because each route reads the object from Postgres first, and ids are never used again. SpiceDB can answer from a cache up to 5 seconds old: a check just after a grant would give a false refusal, so correctness comes first. A ZedToken can make a check faster later (`Check::at_least_as_fresh`).
- **Not kept:** the owner written to SpiceDB before the commit, and deleted again after a failed commit (an unknown commit result leaves an owner with no right); a transactional outbox (a worker, a refusal just after a create, and one write for each resource).

## 21. An unknown resource gives `PERMISSION_DENIED`

- **Decision:** a route on an unknown object gives the same answer as a route without the permission. `GetWorkspace`, `GetProject`, `DeleteProject`, `Grant` and `Revoke` read the object from Postgres first: no row gives `AccessError::denied(&check)`, the refusal of the gate, with the same message. A route on an unknown workspace fails its check, because no owner and no role exist. The account routes give the same answer for a known and an unknown email (see 33).
- **Why:** a caller does not learn which ids or emails exist.

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

- **Decision:** each route that verifies a token checks its own permission: the RPC name in snake case, on the object of the route. `CreateProject` checks `create_project` on the workspace. `DeleteProject` checks `delete_project` on the project and on its workspace. A route of the caller itself checks its permission on the caller: `ListProjects`, `ListWorkspaces`, `CreateWorkspace`, `AcceptInvitation` and the routes of `ProfileService`. A route permission uses a group: `manage`, `work`, `read` in a workspace, `delete`, `edit`, `view` on a project, `nil` on the caller. A feature adds the permissions of its routes on the workspace with `extend Workspace as <Feature>Holder`, on the caller with `extend Actor as <Feature>Caller`, and on the object only when one object can be shared. The gate refuses a check of a permission that is not the route permission. After the route, the server refuses a success when the route verified a token and passed no check of its route permission: it answers `PERMISSION_DENIED` and logs an error.
- **Why:** a role can give one route and not the others: a quota reader reads quotas and creates nothing. Groups keep the common case short, and a new role changes the schema, not the code. A route that forgets its check, or that checks a group, fails in the tests of its success.
- **Not kept:** checks of groups in the routes (a new role gives all the routes of the group, or a change of code); a list of route permissions in a table (a second source next to the schema); a stop before the route runs when a check is missing (the server does not know the object of the route before the route reads the request). A route that must also check the permission of another route needs a new decision.

## 29. The owners of a workspace in Postgres, the roles in SpiceDB

- **Decision:** Postgres holds the containers and their owners: `workspace_owners`, and the `workspace_id` of each resource. SpiceDB holds only the roles that a person gives on purpose: in a workspace, or on one object. The gate finds the rights of a request in this order: the caller is the object itself (a route on the caller), or an owner of the workspace, with no SpiceDB call; else one `CheckBulkPermissions` call asks for the route permission on the object and on its workspace. A role in a workspace goes down to all that is in it. A check of an object in a workspace names it: `Project::DELETE_PROJECT.on(id).within(workspace)`.
- **Why:** a creation or a delete is one Postgres transaction, with no write to SpiceDB and no undo. The owners of a workspace never lose access, even with SpiceDB down. SpiceDB stores only the roles, not one relationship for each resource. The workspace is business data: lists, quotas under concurrency, foreign keys and hooks read it in SQL. A role on one object stays possible, for precise sharing.
- **Not kept:** the container in SpiceDB (a SpiceDB write at each creation, no sorted lists, no quota lock, no foreign key); an owner on each resource (see 32).

## 30. Roles given with `AccessService`

- **Decision:** `AccessService` gives a role with `Grant` and takes it back with `Revoke`. A request names an object (`kind` and `id`), a role and an actor. The edition has a registry of the roles that a person can give: a plugin registers a kind with `.grants(Project::GRANT, Project::REVOKE, Some(service::find_workspace))`, then each role with `.role(Project::VIEWER)`. An edition adds a role to a core kind, for example `.role(QuotaHolder::QUOTA_READER)`. The route permissions are `grant` and `revoke` on the object, with its workspace from Postgres. They are `admin`: only the owners of the workspace and the direct admins give or take back roles. `Grant` needs an actor that exists. A `Revoke` of a role that the actor does not have is not an error. An unknown kind, a bad id and a name that is not a registered role are request errors.
- **Why:** one service for all kinds, and only the roles of the registry: a permission, or a relation that a plugin did not register, cannot be written. The plugin that owns a kind tells how to find its workspace, so the check is the same as on the other routes.
- **Not kept:** a `Grant` RPC in each service (the same code in each feature); a grant written as text (the typed `Relation` gives the kind and the name).

## 31. A list shows what the caller can see

- **Decision:** `ListProjects` returns the projects of the workspaces that the caller owns, of the workspaces where it has a role that lists them, and each project shared with it. `ListWorkspaces` returns the workspaces that the caller owns and the workspaces where it has a role. Postgres gives the owned workspaces. SpiceDB reverse lookups (`LookupResources`, fully consistent) give the others. `workspace_id` is a filter: it causes no refusal. If a lookup fails, the list shows the owned ones only, and the server logs a warning.
- **Why:** a project shared with an actor must be in its list, or the share is of no use. The owner path stays in Postgres, so an owner lists its projects with no SpiceDB. With a share of one project, a caller can see some projects of a workspace and not the others: the filter returns the visible ones.
- **Not kept:** an error when SpiceDB does not answer (the owner path does not need SpiceDB). A caller with very many roles makes one long lookup: a cursor on the lookup is a later change.

## 32. Workspaces

- **Decision:** a workspace is the top-level container: `workspaces (id, name, created_at)`. Each project and each agent is in one workspace. Its owners are users, in their own table (see 38). A new user gets a personal workspace, and becomes its owner, in the transaction of its creation. A user creates more workspaces with `CreateWorkspace`, and becomes their first owner. A person joins a workspace by an invitation (see 37) or a role from `Grant`.
- **Why:** a resource belongs to its team, not to a person. The owners keep a root that never loses access. Quotas, lists and later the SSO of a company have one natural scope. The name follows Slack and Linear.
- **Not kept:** an owner on each resource (the owner leaves, the resource stays); folders between the workspace and the projects (a later layer: the gate chain can grow); the name "organization".

## 38. The owners of a workspace are a relation

- **Decision:** `workspace_owners (workspace_id, user_id, created_at)` holds the owners, with the primary key `(workspace_id, user_id)`. A workspace has one owner or more. `WorkspaceService` changes them with `AddOwner` and `RemoveOwner`, and reads them with `ListOwners`. Only an owner changes the owners: `add_owner` and `remove_owner` are `nil` in the schema, so no role gives them. `ListOwners` needs `read`. An owner can take out another owner or itself. `RemoveOwner` locks the row of the workspace, deletes, counts, and refuses with `LAST_OWNER` when no owner stays. A second `AddOwner` of the same user changes nothing, and so does a `RemoveOwner` of a user that is not an owner. Only a user can be an owner: an agent or an unknown id gives `NOT_FOUND`.
- **Why:** an owner is a relation between a person and a workspace, not an attribute of the workspace. A column gives a workspace to one person only, and a team must not depend on one person. The owners stay in Postgres for the reasons of 29: a creation is one transaction, and the owners keep access with SpiceDB down. The last-owner rule holds under concurrent removals for the reason of the Enterprise project limit: each removal takes the lock of the workspace first, and in READ COMMITTED the count after the lock sees each removal that a previous holder committed. An addition does not take the lock: it can only cause a refusal, never a workspace with no owner. Without the lock, `two_owners_cannot_take_each_other_out` failed on 3 runs of 3.
- **Not kept:** `workspaces.owner_id` (one person only); an `owner` relation in SpiceDB (a SpiceDB write at each creation, see 29); `owner` as a role of `AccessService` (two stores behind one API); a group or an agent as an owner (a later change: a table of its own, or a principal type).

## 33. Accounts in Scylla

- **Decision:** Scylla keeps its own users: `users` (profile and status `pending`, `active` or `disabled`), `user_emails`, `passwords` (Argon2id, off the async threads), and `external_identities` (provider and subject). One user has many ways to sign in. `SignUp` follows `SCYLLA_SIGN_UP_MODE`: `open`, `invite_only` (the default) or `closed`. The CLI `user create` makes a first admin. A sign-up sends a one-time link that verifies the email, and an invitation link verifies it too. A password has 15 to 128 characters and no other rule (NIST SP 800-63B for a single factor). A wrong password and an unknown email get the same answer. A known email at sign-up gets the same answer, and its owner gets a mail. 5 failures lock the account for a time that grows to one hour. A reset link works one hour and closes each session. The database keeps only the hashes of the one-time tokens.
- **Why:** a self-hosted install needs accounts with no other service, and a company can still use its identity provider (see 35). The rules come from OWASP and NIST: no enumeration, slow hashes, short links, and a lock that does not tell which accounts exist.
- **Not kept:** an identity provider as the only way (one more heavy service for each install); a headless identity service next to Scylla (one more service and database). Not in this change: MFA, passkeys, magic links, personal tokens, a check against breached passwords, and a limit for each IP address.

## 34. Sessions

- **Decision:** each sign-in of a person opens a session: `sessions (id, user_id, refresh_hash, methods, expires_at, revoked_at)`, for `SCYLLA_SESSION_TTL_DAYS` (30). The refresh token is `<session id>.<secret>`, and the database keeps the hash of the secret. `Refresh` gives a new token and a new secret, and the old secret stops. An old secret that comes again closes the session. Each access token of a person names its session (`session(id)` in the first block). The gate refuses a token of a closed or expired session, or of a disabled user, in the query of the revocations. A browser keeps the refresh token in an HttpOnly cookie `scylla_session` (`SameSite=Lax`, `Secure` with HTTPS), and a refresh with the cookie needs an `Origin` of `SCYLLA_CORS_ORIGINS`. `SignOut`, `RevokeSession`, and a reset or a change of the password close sessions at once.
- **Why:** a short access token keeps the offline check of Biscuit (see 7), and the session gives a way to stop a person at once. The rotation and the reuse check find a stolen refresh token. A cookie keeps the refresh token away from the scripts of the page.
- **Not kept:** long-lived access tokens (no stop before their end); a refresh token in the storage of the browser (the scripts of the page can read it). Two tabs that refresh at the same time can close the session: a grace time for the old secret is a later change.

## 35. Sign-in with an identity provider

- **Decision:** the identity providers are rows of `identity_providers`, with a kind and its settings in JSON: `oidc` (any OpenID Connect provider) and `github` (OAuth 2 and the API, for GitHub and GitHub Enterprise Server). The CLI adds and changes them (`provider add-oidc`, `provider add-github`), and their client secrets are encrypted with `SCYLLA_SECRET_KEY` (XChaCha20-Poly1305). A browser opens `/auth/<slug>/start`: the server stores the sign-in for 10 minutes with a state, a PKCE verifier and a nonce, sets the cookie `scylla_flow`, and sends the browser to the provider. `/auth/<slug>/callback` takes the sign-in of the state one time, needs the cookie of the same browser, exchanges the code, checks the ID token and its nonce, opens a session in the session cookie, and goes back to the web app. A return address must be in the web app. These HTTP routes are next to the gRPC routes, outside the gRPC-Web layer, which refuses other HTTP/1.1 requests. A native client gives an ID token to `ExchangeIdToken`, and its issuer names the provider. A new identity makes a user only if the provider allows sign-up, the sign-up mode allows it (an open invitation for the email in `invite_only`), and the provider gives a verified email that no account has.
- **Why:** an operator adds a provider with no new release, and a kind is code in one place. The state, the PKCE verifier, the nonce and the browser cookie stop a forged callback, a stolen code, and a sign-in in the browser of another person. The server keeps the client secret, which the browser must never see.
- **Not kept:** the provider in the settings of the server (one provider only, and a restart for each change); a trait of the extension crate for the kinds (it comes with the first EE kind, SAML).

## 36. No automatic link by email

- **Decision:** an identity of a provider is never linked to an account because the emails match. A new identity with the email of an account is refused (`EMAIL_IN_USE`). The person signs in to the account first, then opens `/auth/<slug>/start?link=true` with its session cookie, and the callback needs the session of the same user. An identity of another user is refused (`IDENTITY_IN_USE`). `UnlinkIdentity` cannot remove the last way to sign in.
- **Why:** a provider can give an email that the person does not own. A link on the email alone gives the account to the person who controls that provider.
- **Not kept:** an automatic link for a verified email (a provider can verify badly); an automatic link for a domain that a workspace owns (a later EE feature, with the proof of the domain).

## 37. Invitations and mails

- **Decision:** an admin of a workspace invites an email with a role (`CreateInvitation`). The link goes to that email only, and works 7 days. A user with this verified email accepts it (`AcceptInvitation`), or signs up with it in the mode `invite_only`. The mails go through the `Mailer` trait of the extension crate: SMTP (`SCYLLA_SMTP_URL`, lettre with rustls), or with no SMTP, a log of the recipient and the subject, never of the link. The dev compose has Mailpit (SMTP 51025, web 58025). A failed mail goes to the log, and the request does not fail.
- **Why:** a link to an email proves that email, so the invited person needs no other check. An edition or a test can give its own mailer.
- **Not kept:** the link in the answer of `CreateInvitation` (the admin could use it); a mail queue (a later change, if mails fail often).

## 39. No compatibility code

- **Decision:** Scylla has no deployed database and no client of an old version, so the code keeps no trace of a previous design. A proto removes a field or an RPC with no `reserved` line and no deprecated mark, and the fields of a message have the numbers 1 to n. `buf.yaml` has no breaking rules. A migration has only `CREATE` statements, one file for each feature: a change of a table changes its `CREATE TABLE`, and `just db-reset` creates the dev database again. `build.rs` makes Cargo rebuild `scylla-core` and `scylla-ee` when a migration changes, because `sqlx::migrate!` embeds the files.
- **Why:** an upgrade path costs code, tests and reading time, and nothing uses it before a first release. The clean migrations give the same tables, columns, constraints and indexes as the old chain: a comparison of the two catalogs found no difference.
- **Not kept:** forward migrations with data moves (they stay in the history for a design that is gone); `reserved` fields (no old client reads them). After a first release, this decision changes: from then on, a change is a new migration.

## 40. The code is grouped by subject

- **Decision:** each crate groups its code by subject: `identity` (accounts, sessions, providers, tokens, actors, agents), `tenancy` (workspaces and their owners, invitations, roles), `projects`, and `platform` (capabilities). `scylla-domain` has `kernel/` (ids, names, pages, kinds, errors, with each type at the crate root), `authz/`, `identity/`, `tenancy/` and `projects.rs`. `scylla-core` has only `lib.rs`, `app.rs`, `config.rs` and `error.rs` at its root, then `access/`, `api/` (server, layers, proto conversions), `cli/`, `infra/` (database, mails, secrets, TLS) and `features/<subject>/<feature>/`. The testkit has one file for each subject, and the API tests one folder for each subject.
- **Why:** a flat folder mixed roles (kernel types and entities, wiring and adapters) and grew with each feature. A subject keeps its features side by side: the account, session, provider and token code changes together. The same split in the domain, the core, the testkit and the tests shows where a new file goes.
- **Not kept:** folders by role only, with the 11 features flat in `features/` (the pile-up comes back); one crate for each subject (the compiler would enforce the borders, but with more crates for the Enterprise build to pin and one migration ledger for each crate). A subject that grows can become a crate later: its folder is already its border.
