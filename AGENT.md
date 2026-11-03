# Agent notes

This is the public repository: the core and the Community edition. `README.md` tells what it is and how to run it. `DECISIONS.md` tells why the code has its shape. This file tells how to write code here. Follow it in each change.

The Enterprise edition is in a separate repository (`../ee` in this POC). Do not change it from here.

## Skills

Load the skill of the task from `.claude/skills/`:

- `add-feature`: a new resource, an RPC, or a field.
- `change-extension-point`: a new hook, context value, authenticator need or builder method in `scylla-extension`.
- `finish-change`: the checklist at the end of each change.

## Structure

- Each crate groups its code by subject: `identity` (accounts, instance admins, sessions, providers, tokens, actors, Apps, agents), `tenancy` (workspaces and their owners, invitations, roles), `projects`, and `platform` (the platform object, capabilities). A new feature goes in the folder of its subject.
- `scylla-domain/src`: `kernel/` (ids and their format, names, pages, kinds, errors: the crate root gives each type), `authz/`, a folder for each subject (`identity/`, `tenancy/`), `platform.rs`, and `projects.rs` with `projects/secrets.rs`.
- `scylla-core/src`: `lib.rs`, `app.rs`, `config.rs` and `error.rs` at the root, then `access/` (the gate), `api/` (the server, its layers, the log of each call, the proto conversions), `cli/`, `infra/` (database, logs, mails, secrets, TLS) and `features/`.
- Never add a file at the root of a crate: put it in the folder of its role or of its subject. When a folder fills with files of different roles or subjects, add a level.
- A feature is one folder `scylla-core/src/features/<subject>/<feature>/`. The feature with the name of its subject stays at the level of the subject (`features/projects/`), and the other features of that subject go below it (`features/projects/secrets/`). A feature folder has only the files that it needs:

  | File | Contents |
  |---|---|
  | `mod.rs` | `mod` and `pub use` lines only |
  | `plugin.rs` | the `Plugin` impl: it registers the services, hooks, authenticators and capability |
  | `grpc.rs` | the tonic handler: authenticate, parse, authorize, call the service, answer |
  | `service.rs` | free functions: begin the transaction, run the hooks, call the store, commit |
  | `store.rs` | the SQL, free functions on `&mut Tx<'_>`, no tonic (a `store/` folder with one file for each table when it grows) |
  | `tests.rs` | unit tests of what the API cannot show |

- The pure types of a feature go in `scylla-domain/src/<subject>/<feature>.rs` (`projects.rs` for the projects, `projects/secrets.rs` for their secrets). No `model.rs`.
- The call order is `grpc -> service -> hooks -> store`. A `grpc.rs` never calls a store.
- Import the module, not the function: `store::insert(...)`.
- Each crate has one error type. The routes return `scylla_core::Error`.
- Keep each file below 300 lines.
- The code and the build files never refer to the Enterprise repository. Its feature names (`quota`, `plan`, `tenant`, `enterprise`) are not in the code.

## Ids, workspaces and access

- Each id is `Id<T>` (`ActorId`, `WorkspaceId`, `ProjectId`). The format is only in `scylla-domain/src/kernel/codec.rs`. Never use `Uuid` outside `to_db()` and `from_db()` in a store. `to_bytes()` gives the bits of an id as associated data of an encryption.
- A workspace is the container of the resources. Each resource has `workspace_id` (its container) and `created_by` (its author). Only a workspace has owners: the rows of `workspace_owners`, one user or more. Never add an `owner_id` to a resource or to a workspace. An actor has no owner. The actor kinds are open: a feature defines its own `KIND`.
- The containers are in Postgres, and SpiceDB holds only roles. A creation or a delete is one Postgres transaction with no SpiceDB write. Never copy a container or another fact of the tables into SpiceDB. A role is its own write.
- The rights of a request: a route on the caller itself passes, an instance admin passes on the platform object, and an owner of the workspace passes, with no SpiceDB call. Else a role must give the route permission on the object or on its workspace (one bulk SpiceDB call).
- A route on the whole instance checks its permission on the platform object: `UserAdminHolder::LIST_USERS.on(Platform::ID)`. Declare it with `extend Platform as <Feature>Holder { permission list_users = nil; }`: only the rows of `instance_admins` pass it. A change that takes out an instance admin calls `admins::keep_an_admin` in its transaction, so the last active admin stays (`LAST_ADMIN`).
- Only an owner changes the owners: `AddOwner` and `RemoveOwner` check `add_owner` and `remove_owner`, which are `nil` in the schema, so no role gives them. `RemoveOwner` locks the row of the workspace and refuses with `LAST_OWNER` when no owner stays.
- Zero trust: each RPC that acts calls `gate.caller(&request)`, then `gate.authorize(&caller, <route permission>.on(id))`. On an object in a workspace, add its workspace from Postgres: `Project::DELETE_PROJECT.on(id).within(workspace)`. An unknown object gets `AccessError::denied(&caller, &check)`, the same answer as a refusal. Each refusal goes through `AccessError::denied` or `AccessError::refused`, which log the event `access.refused` at `WARN` with the actor, the route, the permission and the object. The log puts it on the line of the call. The token check uses the gRPC method of the request as the operation (`OperationLayer` adds it). Only `TokenService`, `AccountService`, health, capabilities and the routes `/auth/*` are public.
- Each route has its own permission: the RPC name in snake case (`CreateProject` gives `create_project`). Declare it on the workspace with `extend Workspace as <Feature>Holder { permission create_project = work; }`, on the caller with `extend Actor as <Feature>Caller { permission list_projects = nil; }`, on the platform with `extend Platform as <Feature>Holder`, and on the object only when one object can be shared (`permission delete_project = delete;`).
- A route permission uses a group: `manage`, `work`, `read` in a workspace, and `delete`, `edit`, `view` on a project. A group bundles routes, and a role (a relation) gives a group. Never check a group in a route: the gate refuses a check of a permission that is not the route permission, and the server refuses the success of a route that verified a token with no passed check of its route permission.
- The permission model is Rust code. `scylla_domain::authz!` declares each SpiceDB definition next to its type, in the SpiceDB schema language. It makes the constants: `Project::DELETE_PROJECT` is a `Permission<Project>`, `Project::VIEWER` is a `Relation<Project, Actor>`. Never write the name of a definition, relation or permission as text.
- `Permission::on(id)` takes an id of its resource. `Relation::of(object, subject)` takes an id of each of its two resources. The compiler stops a wrong id, and a wrong name in a permission expression.
- A creation names its workspace, and the caller needs the route permission there, for example `ProjectHolder::CREATE_PROJECT`.
- A delete removes the roles on the object with `gate.delete_object(&Object::of(id))` after the commit. A failure goes to the log only: a role on a deleted object reaches nothing.
- A plugin registers the model of its types with `EditionBuilder::authz::<Type>()`. Install the plugin of a definition before the plugins that extend it (`Workspaces` first). An edition adds relations and permissions to a core definition with `authz! { extend <Type> as <Holder> { ... } }`, registered after the core.
- `AccessService` (`Grant`, `Revoke`, `ListAccess`, `CheckPermissions`) gives and lists only the roles of the registry. A plugin registers a kind with `.grants(AccessRoutes { grant: <Type>::GRANT, revoke: <Type>::REVOKE, list_access: <Type>::LIST_ACCESS }, service::find_workspace)`, then each role with `.role(<Type>::VIEWER)`. `find_workspace` reads Postgres: the workspace of the object, the workspace itself for a workspace, or `None` for an unknown object. `Grant` refuses an agent of another workspace (`ACTOR_OUTSIDE_WORKSPACE`). An edition adds a role to a core kind with `.role(<Holder>::<ROLE>)`. Only the owners of the workspace and the direct admins give roles: write `permission grant = admin;`.
- A list shows only what the caller can see: the objects of the workspaces that it owns (Postgres), plus the reverse lookups `gate.granted(&caller, <Feature>Holder::LIST_<TYPES>)` (workspaces with a role) and `gate.granted(&caller, <Type>::LIST_<TYPES>)` (shared objects). A filter of the request never causes a refusal. A failed lookup gives a list of the owned objects and a warning in the log.

## Accounts and sessions

- A user is an actor of kind `user` with a row in `users`. Make one with `accounts::create_user` (or `insert_user` in a transaction): it adds the email, the handle, the password hash and the personal workspace. Never insert an actor of kind `user` alone.
- Delete a user only with `accounts::delete_user`: it removes the account rows, the empty workspaces of the user alone, its ownerships and its roles, and keeps the actor. Never delete a row of `actors`.
- A password goes through `features/identity/accounts/password.rs` (Argon2id on a blocking thread). A random token comes from `secret::token()`, and the database keeps `secret::hash(token)` only. A secret that the server must read back is encrypted with `Sealer`: a client secret with `seal`, and the value of a secret of a project with `seal_with` and the bytes of the id of its secret as associated data.
- An account route never tells if an email exists: a known and an unknown email get the same answer. A route that sends a mail to an address that can be unknown answers first, and a task (`tokio::spawn`) makes the link and sends it, at most one each `one_time::COOLDOWN` for one account. A change of the email of a user stops each of its open links (`one_time::cancel_all`). A refusal of an account rule is `Rejection` with a stable code (`EMAIL_IN_USE`, `HANDLE_IN_USE`, `LAST_ADMIN`).
- Each sign-in of a person opens a session (`sessions::open`) with its `Client` (`Client::of_grpc` or `Client::of_http`: the user agent and the IP address), and signs its tokens with `gate.issue(&actor, Binding::Session(session))`. An App gets a token bound to its key: `Binding::Key(key)`. A reset or a change of a password closes sessions with `sessions::revoke_all`.
- An identity of a provider is never linked to an account by its email. A link needs the session of the user (`/auth/<slug>/start?link=true`).
- A mail goes through the `Mailer` of `AccountContext` with `mails::send`. Write it with `Letter` (`accounts/letter.rs`): one text body and one escaped HTML body. A link comes from `AccountSettings::link`, with the token in the fragment. A body can hold a one-time link: never log it.
- A secret never goes to the logs or the database in clear. The server stores public keys and hashes only. No answer holds the value of a secret of a project, and `SecretValue` shows `***` in `Debug`. A route that seals a value gives `SECRET_KEY_MISSING` after its route check when the server has no key.

## Apps and agents

- An App is an actor of kind `app` with a row in `apps`. Make one with `create` of `features/identity/apps/service.rs`, which inserts the actor in the same transaction. `AppId` and `ActorId` have the same bits: convert with `App::actor(id)` and `App::of_actor(id)`, never with `from_db`.
- The server never sees a private key. A key of an App comes from `JoinApp` (with a join code) or from `AddAppKey`. A join code is a `secret::token()`, and the database keeps its hash.
- A delete of an App or a revocation of a key stops the stream of the agent with `Registry::stop`. The registry is in memory: the server is one instance.
- The stream of an agent checks each new token as its start did: `gate.authenticate` with the operation of the stream, the same actor, then `gate.authorize` of the route permission.

## Logs

- `api/request_log/` logs one line for each call: its code, its duration, its actor, its IP address and the cause of a refusal. A route does not log its start, its end or its refusal again.
- A fact that an operator must see is a business event after the commit: `tracing::info!(event = "secret.created", secret = %id, %project, name)`. The name is `<subject>.<action>` in the past tense, and it does not change. Add a new name to decision 59.
- Never log a password, a token, a cookie, the value of a secret, the body or the link of a mail. Mask an email with `logs::mask_email`, and the identifier of a sign-in with `logs::mask_identifier`.
- Do not give an event a field with the name of a key of the JSON line: `ts`, `level`, `event`, `message`, `target`, `rpc`, `code`, `status`, `ms`, `actor`, `actor_kind`, `ip`, `request_id`.
- A task of `tokio::spawn` that logs takes `.in_current_span()`, so its lines have the request id of their call.

## Comments

This project overrides the global "no comments" rule.

- Each `.rs` file starts with one `//!` line that tells what the file is.
- Each item, private ones too, has a `///` of one or two short sentences: what it is or does. The compiler stops a missing doc (`missing_docs`, `clippy::missing_docs_in_private_items`).
- No `# Why` sections and no long argument in the code. A decision and its reason go in `DECISIONS.md`.
- A function body of more than 10 lines that does more than one thing has steps. A blank line separates two steps, and one `//` line at the head of each step tells what the step does: `// Refuse an expired or revoked token.` A step comment is one line. It starts with a capital letter and ends with a period. It tells what the step does, not why and not how.
- No other `//` in a body, except `// given`, `// when`, `// then` in a test. No `//` at the end of a code line.
- Write in ASD-STE100 Simplified Technical English. No em dashes.

## Tests

- First, black-box gRPC tests: one binary per app (`tests/api/main.rs`), one folder per subject (`identity/`, `tenancy/`, `projects/`, `platform/`) and one file per feature.
- A unit test only for what the API cannot show (rollback, hook order) and for pure functions.
- Name: `<rpc>_<expected behavior>`. A `///` line tells the rule under test.
- Body: `// given`, `// when`, `// then` blocks.
- One helper per core RPC in the `scylla-testkit` file of its subject (`identity/`, `tenancy/`, `projects.rs`, `platform.rs`), sent as an actor with `authed`. `rpc.rs` has the request helpers. `app.user()` makes a user with its personal workspace, and `app.workspace_of(user)` returns it. `app.admin()` makes an instance admin. `app.grant_role(actor, Role::on("project", id, "viewer"), to)` gives a role through `AccessService`. `app.visible_projects(actor, workspace)` gives the names in the list of an actor. `app.grant(&[...])` writes relationships with no route, and `app.can(actor, check)` checks a permission. `app.check_permissions(actor, kind, id, &names)` and `app.access_of(actor, kind, id, size)` call `CheckPermissions` and each page of `ListAccess`.
- The test server sends its mails to `app.mails`, with their text and HTML bodies: `app.mails.token(email, "verify-email")` reads the token of the last link. A mail that the server sends after its answer needs `app.mails.wait(email, count).await` first. The API tests have a fake identity provider (`idp.rs`, OIDC and GitHub) and a browser that keeps cookies (`browser.rs`). `TestApp::spawn_without_secret_key` starts a server with no key of the secrets, and `app.sealer` opens a sealed value in a test. `Logs::pretty()` and `Logs::json()` capture the log lines of the server of a test, which runs on the thread of the test.
- Each test makes its own data. `sqlx::test` gives each test a new database, and `TestApp` gives each test a new SpiceDB store.

## Protos

- All ids use `scylla.common.v1.Id`. Lists use `PageRequest` and `PageResponse`.
- Each service, RPC, message and field has a comment (`buf lint`, category `COMMENTS`).
- Each RPC has its own request and response messages.
- No compatibility code: no client of an old version exists. Remove a field or an RPC with no `reserved` line and no deprecated mark. The fields of a message have the numbers 1 to n, with no gap.

## SQL

- No upgrade code: a migration has only `CREATE` statements, one file for each feature. To change a table, change the `CREATE TABLE` of its feature, then run `just db-reset` (the dev data is lost). Never write a data move, a `DROP` or an `ALTER TABLE` for an old schema.
- An id column is `uuid`. A reference to an actor is `REFERENCES actors (id)`, with an index.
- After a SQL change, run `just prepare` and keep `.sqlx/` in the change.

## Dependencies

- Use the standard crate for a solved problem (`clap` for the command line). Do not write it by hand.
- Each dependency is in `[workspace.dependencies]` with all its features. A crate writes only `<name>.workspace = true`.
- Tell the user about each change of version or feature: the Enterprise repository uses the same lines.

## Commands

- `just run`, `just test`, `just lint`, `just prepare`, `just schema`, `just db-reset`, `just user <email> <name>` (password on stdin), `just token <actor id>`. The binary also has `user create --admin` and `user admin grant|revoke <id>` for the instance admins.
- Run cargo with `--workspace` or `-p scylla-ce`. On another crate, Cargo builds tokio and sqlx again.

## Pitfalls

- The database is on port 55432. SpiceDB is on 55051, the SpiceDB of the tests on 55052, and Mailpit on 51025 (SMTP) and 51080 (web). These ports are outside the ranges that Windows keeps for Hyper-V (for example 57950-58049): a port in such a range cannot be bound. Port 5432 and the container `scylla-postgres` belong to another project. Do not touch them.
- Use the latest tools: `cargo install sqlx-cli --locked --features sqlx-toml`, and `npm install -g @bufbuild/buf`. Remove an old version before you install a new one.
- The IDE builds in the same target folder while you edit. If a build sees old code of a path crate (for example in `../ee`), run `cargo clean -p <crate>` there.
- A clap `Args` struct needs a unique `#[group(id = ...)]`. The test `cli_is_valid` finds a conflict.
- On Windows, many full test runs in a short time can use all the local ports. A closed connection keeps its port for 4 minutes (`TIME_WAIT`), and Hyper-V keeps blocks of the dynamic range. The error is `AddrInUse` (10048) when a test connects to the database. It is not a bug of the code. Wait some minutes, or set `TcpTimedWaitDelay` to 30 (a setting of the machine, for an admin).
