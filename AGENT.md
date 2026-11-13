# Agent notes

This is the public repository: the core and the Community edition. `README.md` tells what it is and how to run it. `DECISIONS.md` tells why the code has its shape. This file tells how to write code here. Follow it in each change.

The Enterprise edition is in a separate private repository, `scylla-ops/scylla-ee`. Do not change it from here. The protos are in `scylla-ops/scylla-protos`, a git submodule at `proto`.

## Skills

Load the skill of the task from `.claude/skills/`:

- `add-feature`: a new resource, an RPC, or a field.
- `change-extension-point`: a new hook, context value, authenticator need or builder method in `scylla-extension`.
- `finish-change`: the checklist at the end of each change.

## Structure

- Each crate groups its code by subject: `identity` (accounts, instance admins, sessions, providers, tokens, actors, Apps, agents), `tenancy` (workspaces, invitations, roles), `projects` (projects, secrets, pipelines, jobs), `integrations` (the GitHub and GitLab integration: the connections, the repository links, the events, the reports, the checkout), and `platform` (the platform object, capabilities). A new feature goes in the folder of its subject.
- `scylla-agent/src`: `agent/` (the settings, the errors, the loop of the streams), `config/` (the file of the agent), `identity/` (the key pair, the join, the tokens), `stream/` (the host and one session), `jobs/` (the running jobs, the checkout of the code, the order of the nodes, the attempts, the process groups, the output). It knows only the protos: never the core. It builds on Unix and on Windows: the code of one platform goes behind `cfg(unix)` or `cfg(windows)`, and `jobs/process.rs` keeps the process groups of both.
- `scylla-domain/src`: `kernel/` (ids and their format, names, pages, kinds, errors: the crate root gives each type), `authz/`, a folder for each subject (`identity/`, `tenancy/`), `platform.rs`, and `projects.rs` with `projects/secrets.rs`.
- `scylla-core/src`: `lib.rs`, `app.rs`, `config.rs` and `error.rs` at the root, then `access/` (the gate), `api/` (the server, its layers, the log of each call, the proto conversions), `cli/`, `infra/` (database, logs, mails, secrets, TLS, the clients of the Git providers) and `features/`.
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
- A workspace is the container of the resources. Each resource has `workspace_id` (its container) and `created_by` (its author). Only a workspace has owners: the holders of the built-in role Workspace owner, the rows of `workspace_owners`, one user or more. Never add an `owner_id` to a resource or to a workspace. An actor has no owner. The actor kinds are open: a feature defines its own `KIND`.
- The containers are in Postgres, and SpiceDB holds only roles. A creation or a delete is one Postgres transaction with no SpiceDB write. Never copy a container or another fact of the tables into SpiceDB. A role is its own write.
- The rights of a request: a route on the caller itself passes, an instance admin passes on each object, and an owner of the workspace passes, with no SpiceDB call. Else a role that is a row gives the route permission through the catalog (one SQL query), or a role of the schema gives it on the object or on its workspace (one bulk SpiceDB call).
- A route on the whole instance checks its permission on the platform object: `UserAdminHolder::LIST_USERS.on(Platform::ID)`. Declare it with `extend Platform as <Feature>Holder { permission list_users = nil; }`: the rows of `instance_admins` pass it, and a role of the platform level gives it through the catalog. A change that takes out an instance admin calls `admins::keep_an_admin` in its transaction, so the last active admin stays (`LAST_ADMIN`).
- Only an owner or an instance admin changes the owners: `Grant` and `Revoke` of the role `owner` check their route permission, then `manage_owners` with `Gate::permissions`. It is `nil` in the schema and not in the catalog, so no other role gives it. `Revoke` locks the row of the workspace and refuses with `LAST_OWNER` when no owner stays. An invitation does not carry this role.
- Zero trust: each RPC that acts calls `gate.caller(&request)`, then `gate.authorize(&caller, <route permission>.on(id))`. On an object in a workspace, add its workspace from Postgres: `Project::DELETE_PROJECT.on(id).within(workspace)`. An unknown object gets `AccessError::denied(&caller, &check)`, the same answer as a refusal. Each refusal goes through `AccessError::denied` or `AccessError::refused`, which log the event `access.refused` at `WARN` with the actor, the route, the permission and the object. The log puts it on the line of the call. The token check uses the gRPC method of the request as the operation (`OperationLayer` adds it). Only `TokenService`, `AccountService`, health, capabilities, the routes `/auth/*`, the webhook route `POST /webhooks/{id}`, the webhook routes of the Git providers (`POST /integrations/github/webhook`, `/integrations/github/{app}/webhook` and `/integrations/gitlab/{link}/webhook`) and the schema `GET /schemas/pipeline-v1.json` are public. A webhook route checks the signature of the body before any write.
- A new route permission goes in the catalog (`scylla-domain/src/tenancy/catalog/`), or in the list of the test `each_route_of_a_role_is_in_the_catalog` if no role can give it. A custom role is a row (decision 86): never add a relation to the schema for it.
- A permission of the catalog depends on each permission that its pages and actions in the web app need, also in another area (`depends_on`). `catalog::held` adds them and what they depend on. It also adds `workspaces.view` to a role of the platform with a permission of a workspace or of a project, and the implicit permissions of the level. `complete` calls it at each save. Each read of a role that is a row calls it again: the gate (`held_in_rows`), the lookups (`catalog::giving`), the role rows and the effective access. Thus a new dependency reaches the saved roles with no data move. When a page or an action of the web app calls a route with no check in the web app, add it to the table of `catalog/pages.rs`: its test finds a role that opens the page and misses a route.
- Each route has its own permission: the RPC name in snake case (`CreateProject` gives `create_project`). Declare it on the workspace with `extend Workspace as <Feature>Holder { permission create_project = work; }`, on the caller with `extend Actor as <Feature>Caller { permission list_projects = nil; }`, on the platform with `extend Platform as <Feature>Holder`, and on the object only when one object can be shared (`permission delete_project = delete;`).
- A route permission uses a group: `manage`, `work`, `read` in a workspace, and `delete`, `edit`, `view` on a project. A group bundles routes, and a role (a relation) gives a group. Never check a group in a route: the gate refuses a check of a permission that is not the route permission, and the server refuses the success of a route that verified a token with no passed check of its route permission.
- The permission model is Rust code. `scylla_domain::authz!` declares each SpiceDB definition next to its type, in the SpiceDB schema language. It makes the constants: `Project::DELETE_PROJECT` is a `Permission<Project>`, `Project::VIEWER` is a `Relation<Project, Actor>`. Never write the name of a definition, relation or permission as text.
- `Permission::on(id)` takes an id of its resource. `Relation::of(object, subject)` takes an id of each of its two resources. The compiler stops a wrong id, and a wrong name in a permission expression.
- A creation names its workspace, and the caller needs the route permission there, for example `ProjectHolder::CREATE_PROJECT`.
- A delete removes the roles on the object with `gate.delete_object(&Object::of(id))` after the commit. A failure goes to the log only: a role on a deleted object reaches nothing.
- A plugin registers the model of its types with `EditionBuilder::authz::<Type>()`. Install the plugin of a definition before the plugins that extend it (`Workspaces` first). An edition adds relations and permissions to a core definition with `authz! { extend <Type> as <Holder> { ... } }`, registered after the core.
- `AccessService` (`Grant`, `Revoke`, `ListAccess`, `CheckPermissions`) gives and lists only the roles of the registry. A plugin registers a kind with `.grants(AccessRoutes { grant: <Type>::GRANT, revoke: <Type>::REVOKE, list_access: <Type>::LIST_ACCESS }, service::find_workspace)`, then each role with `.role(<Type>::VIEWER)`. `find_workspace` reads Postgres: the workspace of the object, the workspace itself for a workspace, or `None` for an unknown object. `Grant` refuses an agent of another workspace (`ACTOR_OUTSIDE_WORKSPACE`). An edition adds a role to a core kind with `.role(<Holder>::<ROLE>)`. Only the owners of the workspace and the direct admins give roles: write `permission grant = admin;`. The role `owner` is not in this registry: `grants/owner.rs` handles it.
- A list shows only what the caller can see: the objects of the workspaces that it owns (Postgres), plus the reverse lookups `gate.granted(&caller, <Feature>Holder::LIST_<TYPES>)` (workspaces with a role) and `gate.granted(&caller, <Type>::LIST_<TYPES>)` (shared objects). A filter of the request never causes a refusal. A failed lookup gives a list of the owned objects and a warning in the log.
- Each change that a user or a client can see adds a line to `CHANGELOG.md`, under `[Unreleased]`.
- The server has one port for the web app, gRPC, gRPC-Web, the JSON API, the browser routes and the webhooks. Do not add a second port. The web app is in the binary (`rust-embed`, from `web/dist`), and `SCYLLA_WEB_DIR` replaces it with a folder.
- A read of the people of a workspace or of a project shows only the roles on the workspace and on its projects, for each caller. It shows no role of the instance. Only the routes of the platform object show the instance admins (decision 92).

## Accounts and sessions

- A user is an actor of kind `user` with a row in `users`. Make one with `accounts::create_user` (or `insert_user` in a transaction): it adds the email, the handle, the password hash and the personal workspace. Never insert an actor of kind `user` alone.
- Delete a user only with `accounts::delete_user`: it removes the account rows, the empty workspaces of the user alone, its ownerships and its roles, and keeps the actor. Never delete a row of `actors`.
- A password goes through `features/identity/accounts/password.rs` (Argon2id on a blocking thread). A random token comes from `secret::token()`, and the database keeps `secret::hash(token)` only. A secret that the server must read back is encrypted with `Sealer`: a client secret with `seal`, and the value of a secret of a project with `seal_with` and the bytes of the id of its secret as associated data.
- An account route never tells if an email exists: a known and an unknown email get the same answer. A route that sends a mail to an address that can be unknown answers first, and a task (`tokio::spawn`) makes the link and sends it, at most one each `one_time::COOLDOWN` for one account. A change of the email of a user stops each of its open links (`one_time::cancel_all`). A refusal of an account rule is `Rejection` with a stable code (`EMAIL_IN_USE`, `HANDLE_IN_USE`, `LAST_ADMIN`).
- Each sign-in of a person opens a session (`sessions::open`) with its `Client` (`Client::of_grpc` or `Client::of_http`: the user agent and the IP address), and signs its tokens with `gate.issue(&actor, Binding::Session(session))`. An App gets a token bound to its key: `Binding::Key(key)`. A reset or a change of a password closes sessions with `sessions::revoke_all`.
- An identity of a provider is never linked to an account by its email. A link needs the session of the user (`/auth/<slug>/start?link=true`) or the password of the account (`LinkPendingIdentity`, decision 90). Each new link and each unlink sends a mail, and a mail that fails stops nothing (decision 91). A browser route of a sign-in answers a failure with a code on the page `sign-in-error` of the web app, and never with data of the person in the URL.
- A mail goes through the `Mailer` of `AccountContext` with `mails::send`. Write it with `Letter` (`accounts/letter.rs`): one text body and one escaped HTML body. A link comes from `AccountSettings::link`, with the token in the fragment. A body can hold a one-time link: never log it.
- A secret never goes to the logs or the database in clear. The server stores public keys and hashes only. No answer holds the value of a secret of a project, and `SecretValue` shows `***` in `Debug`. A route that seals a value gives `SECRET_KEY_MISSING` after its route check when the server has no key.
- The state of the web app for a user is in `UserStateService`, with the limits of decision 100. Its routes act on the caller, and the server does not read the values. A value never holds a secret or a token. A new value of the web app needs no change here.

## Apps and agents

- An App is an actor of kind `app` with a row in `apps`. Make one with `create` of `features/identity/apps/service.rs`, which inserts the actor in the same transaction. `AppId` and `ActorId` have the same bits: convert with `App::actor(id)` and `App::of_actor(id)`, never with `from_db`.
- The server never sees a private key. A key of an App comes from `JoinApp` (with a join code) or from `AddAppKey`. A join code is a `secret::token()`, and the database keeps its hash.
- A delete of an App or a revocation of a key stops the stream of the agent with `Registry::stop`. The registry is in memory: the server is one instance.
- The stream of an agent checks each new token as its start did: `gate.authenticate` with the operation of the stream, the same actor, then `gate.authorize` of the route permission.

## Pipelines and jobs

- A pipeline or a job is an object of a project: its routes are permissions of `PipelineAccess` on the project and of `PipelineHolder` on the workspace. Check them with `pipelines::access::authorize`: an unknown object and a refusal get the same answer, which names the object.
- A job keeps a snapshot of its nodes and the state of each node in JSON (`pipelines::json`). Change a job only with `Job::apply` and `Job::end`, in a transaction that locks its row (`jobs::find(tx, id, true)`), then `jobs::save`. Each change of the state of a job calls `reports::job_changed` in the same transaction, after the write.
- A change of a job that an agent holds tells its stream after the commit: `Registry::cancel` for a job to stop, `Registry::wake_all` for a free job. A lost command is safe: each later message of the agent about the job gets a cancel.
- The places of the nodes are in `pipeline_layouts`, never in `Node` (decision 101): a field of `Node` goes into each job snapshot, and a drag would change the version of the pipeline.
- Times of a job are in whole microseconds (`runs::now`), as the database keeps them.
- A job of a trigger has the runner of the triggers of its workspace as its author (`fire::runner`), an actor of kind `trigger_runner` with no key and no role. Start a job with `pipelines::service::start` and a `StartJob` (a run, a fire of a trigger or a new run), which checks the secrets and adds the variables of a trigger. A job with a source keeps its commit in `job_sources`, in the transaction of the job. A job from a fork waits for an approval (`awaiting_approval`). At the dispatch, each node gets the `SCYLLA_GIT_*` variables of the source, not masked, and a job from a fork gets an empty value for each secret.
- A project takes its pipelines from one source: the editor, or the files of its repository (decision 108). A file pipeline and an archived pipeline are read only: a change or a delete gives `PIPELINE_READ_ONLY`, and a save of the places works. A list of the pipelines hides the file pipelines of a project of the editor. A trigger of an archived pipeline, or of a file pipeline whose file left the default branch, does not fire. The file pipelines change only through `files::sync_default_branch` and `files::set_source`.

## Logs

- `api/request_log/` logs one line for each call: its code, its duration, its actor, its IP address and the cause of a refusal. A route does not log its start, its end or its refusal again.
- A fact that an operator must see is a business event after the commit: `tracing::info!(event = "secret.created", secret = %id, %project, name)`. The name is `<subject>.<action>` in the past tense, and it does not change. Add a new name to decision 59.
- A fact for the notifications goes through `notifications::emit`, in the transaction of the write. Add its kind in `features/notifications/kinds.rs` (the kinds of the Git integration are in `features/integrations/plugin.rs`) and a text for it in the web app. A parameter is a short text: never a sentence, `jobs.error`, a secret, a token, a link with a token, or the email of another person. A new kind must say who gets a row (`Audience`) and what a reader needs (`Access`). A kind about the reader alone is `Access::Own`.
- The inbox routes act on the caller, and take no recipient (decision 98). A person gets no row for an own action, except a mandatory kind.
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
- Each service, RPC, message and field has a comment (`just lint` of `scylla-protos`, category `COMMENTS`).
- Each RPC has its own request and response messages.
- A change of a proto is a change of `scylla-protos`: change the files there, run its `just lint`, commit and push there, then move the submodule `proto` to the new commit and commit the new pin with the code that uses it. A new file also goes in the list of `scylla-proto/build.rs`.
- No compatibility code: no client of an old version exists. Remove a field or an RPC with no `reserved` line and no deprecated mark. The fields of a message have the numbers 1 to n, with no gap. One exception: `agents/v1`, `apps/v1` for the join, and `auth/v1` for the exchange. An agent on a machine does not update itself, so these stay additive for one minor version (decision 96).

## SQL

- No upgrade code: a migration has only `CREATE` statements, one file for each feature. To change a table, change the `CREATE TABLE` of its feature, then run `just db-reset` (the dev data is lost). Never write a data move, a `DROP` or an `ALTER TABLE` for an old schema.
- An id column is `uuid`. A reference to an actor is `REFERENCES actors (id)`, with an index.
- After a SQL change, run `just prepare` and keep `.sqlx/` in the change.

## Dependencies

- Use the standard crate for a solved problem (`clap` for the command line). Do not write it by hand.
- Each dependency is in `[workspace.dependencies]` with all its features. A crate writes only `<name>.workspace = true`.
- Tell the user about each change of version or feature: the Enterprise repository uses the same lines.

## Release of the agent

- `.github/workflows/release.yml` runs on a tag `v*`, and the tag must be the version of the workspace. It makes six archives, `SHA256SUMS`, the attestations, and the image `ghcr.io/scylla-ops/scylla-agent` with a signature. A tag with `-` is a pre-release, and it does not change `latest`.
- The server has `install.sh` and `install.ps1` in its binary, and it writes its own version in them. A change of a script reaches a machine only with a new server. A version with no release makes each download fail, so set the version only for a tag. Between two releases, the version stays the version of the last tag.
- `GetCapabilities` gives the version of the server and the image of the agent, so the web app shows the same version as the scripts. `SCYLLA_AGENT_DOWNLOADS_URL` and `SCYLLA_AGENT_IMAGE` replace them for a mirror.
- `agent-install` runs the real path on Linux, macOS and Windows, with the binary of the commit and `--binary`. It does not test the download. A pre-release tag and a clean machine test the download.
- The Enterprise edition takes one CE commit. That commit must have a version with a release: check `.../releases/download/v<version>/SHA256SUMS`.

## Commands

- `just run`, `just test`, `just lint`, `just prepare`, `just schema`, `just db-reset`, `just user <email> <name>` (password on stdin), `just token <actor id>`. The binary also has `user create --admin` and `user admin grant|revoke <id>` for the instance admins.
- Run cargo with `--workspace` or `-p scylla-ce`. On another crate, Cargo builds tokio and sqlx again.

## Git integration

- The integration is core code in `features/integrations/`, with one folder for each part (`apps`, `connections`, `links`, `sources`, `events`, `polling`, `reports`, `checkout`, `merge_blocks`, `files`). The `mount` of a part adds its gRPC services to the builder and returns its public routes and its tasks in a `Part`. `api/serve.rs` mounts all the parts one time, and stops their tasks at the end. A part reads what it needs from `IntegrationContext`.
- Each call of the integration to GitHub or GitLab goes through `infra::vcs`: `ApiClient` with an `Auth`, `Call::list` for the pages, `Call::if_none_match` for an ETag. A failure is a `ProviderError`. Send again only after `RateLimited` (at its time) or `Transient`.
- A call as the GitHub App uses `GitHubApp::jwt`. A call on a repository uses `GitHubApp::installation_token` with the smallest `TokenScope` (repositories and permissions). A GitLab call sends the token of the service account with `Auth::PrivateToken`.
- A token, a key or a webhook secret is a `Hidden`: it never goes to `Debug`, a log line, an answer or a URL. The installation tokens and the deploy tokens stay in memory or in the dispatch of a job, never in the database. A secret that the server keeps is sealed with `seal_with` and the id of its row.
- A webhook route checks the signature with `check_github` or `check_gitlab` before any write.
- A report goes to the provider through the outbox `vcs_reports` (decision 105): `reports::job_changed` records the latest state in the transaction of the change of the job, and the sender task posts it later. Never call a provider in the transaction of a job. A 401, 403 or 404 of a report moves the connection to `needs_attention` and tells the owners and the admins of the workspace.
- The agent fetches the code of a job before its first node (`scylla-agent/src/jobs/checkout.rs`). It gives the header of the dispatch to each git command as `http.<clone URL>.extraHeader`, so only the repository gets it. With git 2.31 or newer, the header is in the variable `SCYLLA_CHECKOUT_HEADER` of the git process (`--config-env`), so it is not in the process list. An older git gets it with `-c`, and the agent logs a warning with no secret. The agent never writes the header in `.git/config`, in a URL or in a log line. At each hello, it reads `git --version`, and it tells the feature `checkout` only when git runs.
- A job with a source and its checkout on has `need_checkout`: only an agent with the feature `checkout` gets it. At the dispatch, `checkout::prepare` makes the token of the job: an installation token of one repository on GitHub, or a GitLab deploy token. The row of a deploy token in `vcs_checkout_tokens` has no token, and the cleanup task of the `checkout` part deletes the token when no job needs it.
- Test a call to a provider against `FakeProvider` of the testkit: `respond` gives the answers of a route, `requests` gives what the server sent, and `fake.options(true)` in `ServeOptions.vcs` gives a server with the test GitHub App. The rows that a route makes later come from the fixtures (`insert_github_connection`, `insert_repository_link`, `insert_job_source`, `insert_git_trigger`). The tests of a part go in `scylla-ce/tests/api/integrations/<part>.rs`, and its helpers in `scylla-testkit/src/integrations/<part>.rs`. A test on the real GitHub.com or GitLab.com is `#[ignore]` and reads the `SCYLLA_LIVE_*` variables.

## Pitfalls

- The database is on port 55432. SpiceDB is on 55051, the SpiceDB of the tests on 55052, and Mailpit on 51025 (SMTP) and 51080 (web). These ports are outside the ranges that Windows keeps for Hyper-V (for example 57950-58049): a port in such a range cannot be bound. Port 5432 and the container `scylla-postgres` belong to another project. Do not touch them.
- Use the latest tools: `cargo install sqlx-cli --locked --features sqlx-toml`. `buf` is a tool of `scylla-protos`. Remove an old version before you install a new one.
- The IDE builds in the same target folder while you edit. If a build sees old code of a crate, run `cargo clean -p <crate>`.
- A clap `Args` struct needs a unique `#[group(id = ...)]`. The test `cli_is_valid` finds a conflict.
- On Windows, many full test runs in a short time can use all the local ports. A closed connection keeps its port for 4 minutes (`TIME_WAIT`), and Hyper-V keeps blocks of the dynamic range. The error is `AddrInUse` (10048) when a test connects to the database. It is not a bug of the code. Wait some minutes, or set `TcpTimedWaitDelay` to 30 (a setting of the machine, for an admin).
