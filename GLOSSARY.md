# Scylla Glossary

Reference for every domain-specific word used across Scylla's code, docs, and UI. Grouped by topic.

## Platform services

### `scylla-ce`
The Community Edition binary, and the process people call "the control plane". Serves everything on one port (`8080`): the compiled-in web UI, the gRPC API (user APIs, app token exchange, and the agent worker stream), its gRPC-Web translation for the browser, and the inbound webhook ingress. Job dispatch and log fan-out are in-process, there is no message broker and no recorder. Config lives in `binaries/scylla-ce/config/*.toml`.

The package is a `main.rs` and nothing else: it loads a configuration, opens the pool and hands both to the `scylla_server::Server` builder, which owns the composition root; it registers no extension (see [Extension point](#extension-point)), so every write runs the pipeline with no hook. The use cases live in `scylla-core`, the access model in `scylla-auth`, the Postgres adapters in `scylla-db`.

The package name, the binary name, the Dockerfile stage name and the config directory path are load-bearing: the image is built with `--target scylla-ce` and `docker-compose.yaml` bind-mounts `binaries/scylla-ce/config`. So is the depth of `scylla-db` and `scylla-core` under `crates/`: `sqlx::migrate!("../../migrations")` and the rust-embed `#[folder = "../../web/dist/"]` resolve against `CARGO_MANIFEST_DIR`. Changing any of them breaks the image build or the container at runtime, not the `cargo` build.

### `scylla-agent`
The worker binary on each machine that runs pipelines. It authenticates as its [App](#app): it exchanges `--app-id` and `--app-secret` for a bearer token. Then it opens the `AgentService` stream of the control plane on port `8080`. It runs each job that it receives as a task, and it starts a node when each of its deps has completed. It reads the stream while the jobs run, so a `CancelJob` frame stops a job at once. It sends the status and the output of each job on the same stream. The open stream is the presence of the agent; there is no heartbeat. On `SIGTERM` it stops its jobs, reports their end, and then closes the stream.

### `scylla-domain`
The shared kernel, and the only Rust code both binaries run. Holds the domain model (entities, value objects, `DomainError`, the DAG planner) plus [`JobEvent`](#jobevent), the command vocabulary the agent reports and the control plane applies.

Not a service, and deliberately dependency-light: `serde`, `chrono`, `nutype`, `derive-where`, `ulid`, `thiserror` and nothing else. It links no database driver, no gRPC stack and no crypto, so an agent can depend on it without pulling in the server's world. Anything that talks to an external system is an adapter and belongs in `scylla-core` or `scylla-db`. The crate has no Cargo features at all.

### `scylla-proto`
Library crate with the Rust bindings of the `.proto` files and the conversions to the domain types. The `.proto` files are in `crates/scylla-proto/proto/`, a git submodule of [`scylla-ops/scylla-protos`](https://github.com/scylla-ops/scylla-protos). The web UI pins the same repository with its own submodule.

### `postgres`
Primary datastore. PostgreSQL 18 in the compose stack, listening on port `5432`. Schema is managed by versioned SQL files in `migrations/` applied via `sqlx::migrate!` at boot. The offline query cache lives in `.sqlx/` so Docker builds can compile without a live database.

## Domain entities

### Organization
Top-level tenant. Carries a `name`, optional `description`, and an `is_active` flag. The name is a label: two organizations can have the same name. Owns projects. People reach it by holding a grant on it or on one of its projects; there is no membership table.

### Member
A user that holds a grant on an [organization](#organization) or on one of its projects. An app is not a member. `OrganizationRepository::list_principals` lists the members, `count_members` counts them and `has_member` tells if a user is a member. In the Community Edition, a member is added only with `GrantService.CreateGrant` (`manageOrgGrants` on the organization), on a user that exists. The web dialog "Add a member" picks the user in the user directory, thus it needs `listUsers`, which only a system administrator has. The Enterprise edition adds [invitations](#invitation).

### Invitation
Not in the Community Edition. The Enterprise edition adds invitations by email, with its own tables: an organization administrator sends an invitation, and the person joins the organization with a new account or with the account of that email. The migration `20261009120000` removed the invitation tables of the Community Edition.

### Project
A unit of work inside an organization. Carries a `name`, optional `description`, `organization_id`, and `is_active` flag. Owns pipelines and jobs. Being on a project means holding a grant scoped to it.

### Pipeline
A directed acyclic graph (DAG) of **nodes** describing what to run. A pipeline is a blueprint — running it produces a **job**.

### Node (`PipelineNode`)
One step in a pipeline. Carries a `command`, `args`, and a list of `deps` (other node IDs it depends on). Nodes must form a DAG — cycles are rejected at creation time.

### Job
A single execution of a pipeline. Carries a `JobStatus`, the nodes of the pipeline when the job was created, and a `JobNode` for each of these nodes, with its state and its times. A change to the pipeline does not change a job that exists. Each write of a job names the version it read.

### JobNode
The runtime record for one pipeline node inside a job. Holds `NodeState`, `started_at`, `finished_at`.

### App
A machine principal owned by an organization (an agent / automation). Identified by an `AppId`; authenticates with a secret (stored hashed) that it exchanges for an app token, and acts under scoped grants — typically the `organization-agent` role on its org. An App with an open agent stream is an online **agent**; "agent" now means an App running the agent binary.

`AppService.ListAppAccess` shows each grant of an App with the names of its organization, project and role. It asks for `readApp` on the App. An unknown App gives an empty list.

### App secret
A secret of an [App](#app). The App exchanges it for an app token. The store keeps only its hash. Each secret has a label that is unique in its App: trimmed, 1 to 64 UTF-8 bytes, no control character (`AppSecretLabel`). `CreateApp` and `AgentAdminService.CreateAgent` make the first secret, with the label `secret_label` of the request. Without `secret_label`, the label is the App name without its control characters, cut to 64 bytes. If no valid label is left, the label is `default` (`DEFAULT_SECRET_LABEL`). `default_secret_label` in `scylla-domain` is the only place of this rule. `AppService.SetAppSecretLabel` changes the label of one secret. It asks for `deleteApp` on the secret, as `RevokeAppSecret` and `SetAppSecretEnabled`. A label that another secret of the App has gives `ALREADY_EXISTS`. The plaintext, the tokens and the open agent stream do not change.

### Trigger runner
The App that a trigger fire runs as. An organization has one trigger runner or none. The server makes it at the first `CreateTrigger` in the organization. It has the kind `trigger_runner`, the name `trigger-runner`, the `organization-trigger-runner` role and no secret. The server finds it by its kind, not by its name. A user cannot give this name to an App, and cannot disable the trigger runner, delete it or add a secret to it.

### Session
An authenticated user session. Carries an opaque `token`, `user_id`, `created_at`, `expires_at`, and `last_active_at`. Created on login; the auth interceptor looks it up by token on each gRPC call and rejects expired sessions. The store keeps only the SHA-256 of a token, as for an app token.

These actions revoke the sessions of a user: `UserService.RevokeUserSessions` ("sign out everywhere"), `ChangePassword`, a [reset link](#reset-link), a deactivation (`SetUserActive`) and a deletion. `ChangePassword` and `RevokeUserSessions` on the caller's own account keep the session of the call open. The interceptor gives that session to the handler (`CallerSession`).

A session also carries its client (`SessionClient`): the user agent and the IP address of the `Login` call that opened it. Both are optional. The server trims the user agent, removes its control characters and keeps at most 512 characters. It keeps an IP address only when the value parses as an IP address. The IP address is the peer address of the connection. When `[server].trust_forwarded_headers` is `true`, it is the first entry of `x-forwarded-for`, else `x-real-ip`, else the peer address. A session that the server opened before it recorded the client has no client. `scylla_core::grpc::session_client` reads the client from a request.

The five-minute rule: a call with a session moves its `last_active_at`, at most one time in five minutes (`session::ACTIVITY_INTERVAL`). The interceptor writes only when the stored value is five minutes old or older, with one conditional `UPDATE`. Thus most calls write nothing, and `last_active_at` can be up to five minutes old. If the write fails, the interceptor logs a WARN and the call continues.

`UserService.ListUserSessions` shows the sessions of a user that have not expired, the most recently active first, without the tokens. `current` marks the session of the call. `UserService.RevokeUserSession` revokes one session. It gives `NOT_FOUND` for an unknown id, a session of another user and an expired session, with the same message. The caller can revoke the session of the call: the caller is then signed out. The list asks for `readUser` on the user, the revoke for `updateUser`; the self rule of the Cedar policies gives both to the user for its own account.

### User
A user account. A user is related to the tenancy tree only through **grants**: holding a role on a scope is what puts them there, so "who is in this organization" and "what may they do" are the same rows (see [Authorization](#authorization)).

In the Community Edition, an account comes from one of two sources: `UserService.CreateUser`, which a system administrator sends, or the [bootstrap](#bootstrap-user). `CreateUser` needs an email. The Community Edition sends no mail: it writes each [reset link](#reset-link) in the server log. The Enterprise edition adds sign-up, invitations, GitHub sign-in and mail.

A user reads its own account with `UserService.GetMe`, changes its password with `ChangePassword` and deletes its account with `DeleteAccount`. An app token cannot call these three RPCs: they give `PERMISSION_DENIED`. A wrong password gives `FAILED_PRECONDITION`, not `UNAUTHENTICATED`, because the web UI signs out on `UNAUTHENTICATED`. `DeleteAccount` keeps the rules of `DeleteUser`: the last holder of `system-admin`, or of `organization-admin` on an organization, cannot delete the account. The message names those organizations. `ListUserAccess` shows each grant of a user with the names of its organization, project and role.

`SetUserActive` sets the `is_active` flag. A user cannot change its own flag. A deactivation that leaves no active user with `system-admin` is refused. An inactive user cannot log in.

### Display name
The name of a [user](#user) that people read. It is optional and not unique. The value is trimmed, has 1 to 100 characters (characters, not bytes) and has no control character. `CreateUser` and `UpdateUser` set it. An empty value in `UpdateUser` removes it.

### Reset link
A single-use link that sets a new password: `<public_url>/reset-password#token=<token>`. The token is 32 random bytes in base64url without padding. It is in the URL fragment, so a request to the server does not carry it. The store keeps only the SHA-256 of the token, in the table `password_resets`. A link expires one hour after the server makes it.

`AuthService.RequestPasswordReset` makes a link for the active account with the email. The answer is the same for an unknown email, an inactive account and a known one. The answer does not wait for the store: a task stores and delivers the link after the answer, and logs a failure at WARN. If the account got a link less than 60 seconds before, the task stores and delivers nothing. `UserService.SendPasswordReset` makes a link for an administrator or for the user itself, without the 60-second rule, and delivers it before the answer.

A new link cancels the earlier links of the account. `ResetPassword` sets the password, marks the link used, cancels the other links and revokes each session of the account. An unknown, used or expired link and an inactive account give one error: "This reset link is not valid". `ChangePassword`, a deactivation and a deletion also cancel the links.

### Reset sender
The port that delivers a [reset link](#reset-link): `PasswordResetSender` in `scylla_core::application`. It has two methods. `delivery()` tells how the server delivers its links (`Mail` or `ServerLog`); it depends on the sender only, never on the account. `send(&PasswordResetMessage)` delivers one link. The message holds the user id, the email, the username, the display name, the link and the expiry time.

The default sender is `LogPasswordResetSender` (`scylla_core::infrastructure`). It writes one INFO line, with the target `scylla_core`, the user id, the username and the link. It never writes the email. An edition replaces it with `Server::password_reset_sender(Arc<dyn PasswordResetSender>)`.

## States & status values

### `JobStatus`
- `Pending`: created. The job waits in the pool, or it is placed on an agent that has not started it.
- `Running`: an agent reported that the job started.
- `Completed`: all nodes finished successfully.
- `Failed`: a node failed, the agent could not run the job or stopped it when it shut down, or the control plane could not dispatch it.
- `Cancelled`: `JobService.CancelJob` ended the job.
- `Orphaned`: the agent stayed disconnected for 60 seconds after its last contact, or it reconnected without the job.

Terminal: `Completed`, `Failed`, `Cancelled`, `Orphaned`. Legal transitions: `Pending → Running | Failed | Cancelled | Orphaned`; `Running → Completed | Failed | Cancelled | Orphaned`. When a job ends, each node that has not finished becomes `Cancelled`.

### `NodeState`
Per-node execution state: `Pending`, `Running`, `Completed`, `Failed`, `Cancelled`, `Skipped`. Terminal: everything but `Pending` and `Running`. Unlike `JobStatus` there is no `Orphaned`; conversely `Skipped` has no job-level equivalent, it marks a node whose dependency failed so it never ran.

### Terminal state
A state from which no transition is possible. A job or node in a terminal state is done.

## Authorization

The vocabulary below is the **single source of truth** for authorization words:
one word per concept, used identically in code, proto, DB, and UI.

**The seven words:** **Permission** (atomic capability) · **Role** (bundle of
permissions) · **Scope** (System/Organization/Project) · **Grant** (P holds X in
scope S) · **Principal** (a grant-holding User\|App) · **Caller** (the request
identity) · **Policy** (advanced Cedar rule). Plus **Resource** (the entity an
action targets). "action" is retired from the domain vocabulary — it survives
only as Cedar's wire term inside the infra adapter.

### Role naming convention
Builtin role names follow **`<scope>-<role>`**, kebab-case, with scope ∈ {`system`,
`organization`, `project`}. The `*_ROLE` constants in
`crates/scylla-auth/src/authz/grant/mod.rs` name them.

**Unified model:** there is ONE authorization mechanism — the **grant** (`grants`), a `(principal, role, scope)` triple linked as a Cedar role-template instance. A "global role" is just a grant on the **System** scope (the tenancy root: `Organization`/`User` are `in [System]`, so a System grant reaches everything). There is no separate `user_roles` table and no `RoleService` — `GrantService.CreateGrant(user, role, GRANT_SCOPE_SYSTEM)` replaces the old `AssignRole`.

Builtin roles, seeded in the `roles` table. The table also holds custom roles:
platform roles that a system administrator creates, and roles that one
organization owns (see Role). `ListGrantableRoles` reads the table.

| Role | Scope | Confers |
|------|-------|---------|
| `system-admin` | System | global super-user: full control over every scope (System is the root) |
| `organization-creator` | System | create organizations, nothing else; the creator becomes the admin of each one |
| `organization-admin` | Organization | owner of an organization and everything beneath it |
| `organization-viewer` | Organization | read every project and run in the organization, change nothing |
| `organization-member` | Organization | belongs to the organization: sees it exists, nothing more |
| `organization-agent` | Organization | machine app scoped to an organization: pull and run its jobs |
| `organization-trigger-runner` | Organization | machine app that fires the organization's triggers: run its pipelines |
| `project-admin` | Project | owner of a project and everything beneath it |
| `project-developer` | Project | build in a project: create, edit and run its pipelines, and cancel their runs |
| `project-viewer` | Project | read a project, its pipelines and its runs, change nothing |
| `project-agent` | Project | machine app scoped to a project: pull and run its jobs |

**No implicit tiers.** Belonging somewhere confers nothing on its own, so every level of access is a role in the table above — down to `organization-member`, which grants only the ability to see that the organization exists. A principal with no grant can do nothing.

### Permission
The atomic capability — a verb on a resource type, e.g. `runPipeline` on a
pipeline. Closed, code-owned catalog: the `Permission` enum
(`domain/permission.rs`), one variant per real enforced capability.
`Permission::key()` is its canonical id (`"runPipeline"`) — the value a role
stores and the Cedar `Action::"…"` eid. A permission cannot be created at runtime
(only the code that enforces it gives it meaning); **roles** are the dynamic part.
The **resource type** a permission targets is *derived* (`Permission::resource()`
→ `ResourceRef::kind()`), never stored or sent on the wire — `read_user` always
targets a user, so naming it would be redundant.

### Scope-pinned permission families (don't merge these)
A few permissions encode a scope in their name on purpose — `manageSystemGrants`
/ `manageOrgGrants` / `manageProjectGrants`, and the `list*` / `list*ByOrganization`
/ `list*ByProject` / `list*ByPipeline` families. Each member is a **distinct Cedar
action** with its own `appliesTo` resource type; that split is the anti-escalation
fence (a single shared action would let one over-broad permit authorize every
scope). They look redundant but are **load-bearing — do not collapse
them into one Cedar action.** The UI presents each family as a single concept (one
"Manage grants", one "List jobs") with a scope selector; the split stays in the
schema only.

### Role
A named bundle of permissions bound to a scope kind, stored in the `roles` +
`role_permissions` tables. A role with no owner is a **platform role**: the
builtin roles, seeded on first boot, and the custom roles of the system
administrators (`manageRoles`). A role with an owner is an **organization role**:
it is organization or project scoped, only its organization sees and grants it,
and its administrators manage it (`manageOrgRoles`). A role has a kind: `admin`
(builtin owner roles only), `member`, or `agent` (apps only). A grant of a role
confers all its permissions within the grant's scope. The live Cedar policy set
is **generated** from these rows (a full-control role — permission `*` — maps to
the unconstrained-action body; any other role lists its permission keys), so
editing a role's permissions changes authorization on the next check.

### Scope
The level a grant/role binds to: `System` (tenancy root), `Organization(id)`, or
`Project(id)`. `Scope` carries the id; `ScopeKind` is its id-free discriminant.
Because every entity is `in` its scope ancestors (Project `in` Org `in` System),
a grant at a scope reaches everything beneath it.

### Grant
"Principal P holds {a role | a single permission} within scope S." The one
authorization mechanism: stored in `grants`, linked into Cedar on the next check. A
direct **permission** grant (e.g. Alice `runPipeline` in Org A) is additive to
P's role-derived permissions.
A grant refers to a principal and a scope that exist: a database trigger
refuses the insert if not. An app gets grants only in the organization that
owns it. A user cannot get an agent role. `check_grantable` (`scylla-auth`) is
the one check of a new grant.

### Principal
A grant-holding actor: a human `User` or a machine `App`. Maps to the
`(principal_kind, principal_id)` columns of `grants` and to the Cedar
`?principal` slot. Distinct from **Caller**: a Caller may also be a Service or
Anonymous, which cannot hold grants.

### Policy
A Cedar rule in the live policy set. They come from exactly two places: the
static base compiled into the binary (`policies.cedar`), and one generated
template per role, instantiated once per grant. There is no runtime-authored
policy. Not a synonym for permission or grant.

### `PermissionService`
Port (`scylla-auth`, `authz/service.rs`) that the `Authorize` stage asks,
through `PermissionAuthorizer`, for each permission an action requires:
`check(caller, Permission) -> DomainResult<()>` (fail-closed, never a bool). The production adapter is **Cedar**-backed (`CedarPermissionService`):
it builds the principal/resource entities, snapshots the live policy set, asks
Cedar, and records an audit-log row.

### Cedar
The authorization engine (AWS Cedar). Scylla generates its policy set from the
model (roles + grants → `permit` policies) plus a small static base
(`policies.cedar`: internal services, self-read).
Resolves the scope hierarchy via `in` and yields the allow/deny decision +
diagnostics.

### Bootstrap user
The initial `admin` account. The `[bootstrap]` section of the control-plane config sets its `username`, `password` and `email`. The email is necessary. The default values in local dev are `admin`, `admin123` and `admin@example.com`. The account gets a `system-admin` grant on the `System` scope (full control over every scope, because System is the tenancy root).

The server runs the bootstrap at each start, as the bootstrap service. It creates the account. If the username or the email exists, it finds the account as follows:

- An account has the configured email. If its username is the configured username, the server makes sure that it has the grant. If not, the start stops with an error: the email belongs to another account.
- No account has the configured email, and the account with the configured username has no email. The server sets the configured email on that account (`UpdateUserEmail`), then makes sure that it has the grant.
- No account has the configured email, and the account with the configured username has another email. The start stops with an error. No account gets the grant.

`GetUserByUsername`, `GetUserByEmail` and `UpdateUserEmail` have no RPC. Only the bootstrap sends them.

### Caller (`CallerContext`)
The identity that made a request, threaded through the authorization layer. Variants: `User(UserId)`, `App(AppId)` (a machine principal / agent), `Service(ServiceIdentity)` (sealed internal identity), `Anonymous`. A Caller that is a `User` or `App` is also a **Principal** (can hold grants); `Service`/`Anonymous` cannot. Each maps to a Cedar entity (`Scylla::User::"…"`, `Scylla::App::"…"`, `Scylla::Service::"…"`); the Cedar `PermissionService` resolves the caller's roles, scoped grants, and tenancy ancestry on each check.

## Pipeline execution concepts

### DAG (Directed Acyclic Graph)
The shape a pipeline's nodes must form. Validated at pipeline creation via cycle detection.

### Topological order
A linear ordering of nodes consistent with dependencies (each node appears after its deps). Scylla never materializes one: [`DagPlan`](#kahns-algorithm) hands out whole batches of ready nodes instead, so independent nodes run in parallel. `BTreeSet` keeps the batch order deterministic.

### Dependency (`deps`)
A prerequisite node ID. A node only becomes runnable once all its deps are in a successful terminal state.

## Agent presence

### Connected (online)
An agent is connected when its [App](#app) holds an open `AgentService` stream to the control plane. The registry keeps the presence in memory; there is no heartbeat. A second stream of the same App replaces the first. Agent listings show it as a `connected` flag.

### Placement
The dispatcher gives the oldest pending job to a connected agent that holds `executeJob` on the pipeline of the job and has no job that has not ended. It reads this from the grants of the agent, so it writes no audit row. The job goes to the open stream of the agent and shows the agent in `Job.Pending.agent_id` until it starts. When that stream closes, the job goes back to the pool if it did not start. A newer stream of the same agent keeps the jobs placed on it.

`JobService.ListOrganizationJobs` with `agent_id` lists only the jobs that the server placed on that agent (`jobs.agent_app_id`), at each state. A job in the pool is not in this list.

## Networking & protocol

### gRPC
Primary transport between API and internal services. Defined in `.proto` files in `crates/scylla-proto/proto/` (the `scylla-protos` submodule), which is the single include root. Each file's path below that root **is** its package: `scylla/job/v1/job.proto` declares `package scylla.job.v1`. Packages carry a version suffix from day one, so a breaking change means a new `v2` directory beside `v1`, never an edit in place. `buf lint` and `buf breaking` enforce both (`just proto-lint`, `just proto-breaking`).

Two packages are deliberate leaves that others may import: `scylla.common.v1` (id wrappers, `Email`, pagination) and `scylla.exec.v1` (the step contract shared by pipelines and agents). Feature packages reference across contexts by id. One exception: `scylla.user.v1` imports `scylla.auth.v1` for `PasswordResetDelivery` and `scylla.authz.v1` for `ScopeRef`.

### gRPC-Web
Browser-compatible variant of gRPC, spoken by the web UI. Served by the control plane through `tonic-web`.

### Tonic
Rust gRPC server/client framework used by all backend services.

### Prost
Protobuf code generator used by Tonic. Converts `.proto` → Rust structs.

### Auth interceptor
Async Tonic interceptor (`crates/scylla-core/src/grpc/middleware/auth_interceptor.rs`). Reads the `authorization: Bearer <token>` metadata and resolves it to a principal: a user session (`SessionRepository`) or, failing that, an app token (`AppTokenRepository`). Rejects expired or unknown tokens with `Unauthenticated` and attaches an `AuthContext { caller }` (`CallerContext::User` or `CallerContext::App`) to the request extensions. For a user session, it also attaches `CallerSession`, the id of that session. `caller_session(&request)` reads it. For a user session, it also applies the five-minute rule of the [session](#session) (`auth::record_activity`): this is the only write of the interceptor.

## Identifiers

### Entity IDs
All domain IDs (`UserId`, `OrganizationId`, `ProjectId`, `PipelineId`, `JobId`, `JobLogId`, `SessionId`, `AppId`, `AppTokenId`, `AppCredentialId`, `SecretId`, `TriggerId`) are opaque string newtypes generated as lowercased ULIDs via `::generate()`. They also accept external strings via `::new(...)`. Generated by the `define_id!` macro in `domain/ids.rs`; they carry no `sqlx` integration because every query binds them as `&str` through `as_str()`.

### `NodeId`
Caller-supplied ID for each pipeline node. Must be unique within its pipeline. Validated as a lowercase ASCII alphanumeric string plus `-` / `_`, max 128 chars.

## Infra & dev

### `docker-compose.yaml`
Defines the stack (`postgres`, `scylla-ce`). The web UI ships inside the `scylla-ce` image, so there is no separate frontend service. Agents are installed out-of-band (see [`scylla-agent`](#scylla-agent)), not in this stack.

### `justfile`
Task runner recipes: `just up`, `just down`, `just logs`, `just release <version>`, etc.

### `config/*.toml`
Per-environment config for `scylla-ce` (under `binaries/scylla-ce/config/`): `local.toml` (host-native dev), `docker.toml` (compose), `prod.toml` (production).

## Architecture terms

The hexagon is split across several crates. The model sits in the kernel; the use cases and inbound adapters in `scylla-core`; the access model in `scylla-auth`; the Postgres adapters in `scylla-db`; the composition root in `scylla-server`; the edition binaries on top.

`crates/scylla-domain/src/` (the kernel, shared with the agent):

- `domain/<aggregate>.rs` and `domain/<aggregate>/`: the entities and their value objects.
- `domain/pipeline/dag.rs`: the DAG planner, see [Kahn's algorithm](#kahns-algorithm).
- `domain/errors.rs` — `DomainError` / `DomainResult`.
- `domain/caller.rs` — [`CallerContext`](#caller), the identity every use case and hook receives.
- `domain/job/event.rs`: [`JobEvent`](#jobevent).

`crates/scylla-core/src/`:

- `application/<feature>/repository.rs`: the ports; `mod.rs`: the use case struct; `commands.rs` / `queries.rs`: the actions and their `Run` impls.
- `infrastructure/`: the adapters that need no database, `messaging/*` (in-process dispatch and log fan-out), `services/argon2_hash_service.rs`, and the rest.
- `grpc/` and `rest/` — the inbound adapters.
- `config.rs`: the server configuration and its TOML loader.

`crates/scylla-auth/src/`: `authz/` (the permission, role, grant and visibility ports and types), `audit.rs`, and `cedar/` (the Cedar adapter with its schema and policies).

`crates/scylla-db/src/`: `postgres/<aggregate>/` (one `Pg…Repository` per aggregate) and `pool.rs` (the pool and the embedded migrations).

`crates/scylla-server/src/`: `server.rs` (the `Server` builder, the public entry point), `startup.rs` (the `Services` struct and `init_services`, the one place that names every concrete adapter, plus `run_server`; all crate-private) and `cli.rs` (the command line, config loading and tracing setup shared by the edition binaries).

`crates/scylla-extension/src/`: the action pipeline, see [Extension point](#extension-point) and `crates/scylla-extension/ARCHITECTURE.md`.

Every crate re-exports the kernel's `domain` module, so `crate::domain::...` names the model from anywhere even though it lives in another crate. That is the only re-export of its kind: the access model is reached as `scylla_auth::authz`, and the Postgres adapters import the ports they implement from `scylla_core::application`.

### Port
A trait describing something the use cases need from the outside world (persistence, hashing, permission checks), declared next to its use case in `application/<feature>/`. Examples: `PipelineRepository`, `HashService`. `PermissionService` is declared in `scylla-auth`.

### Adapter
A concrete implementation of a port. **Driven** adapters, the ones the use cases call out to, live in the crate that owns their dependency: `scylla-db/src/postgres/*` for repositories, `scylla-auth/src/cedar/` for the policy engine, `scylla-core/src/infrastructure/` for the rest (hashing, encryption, cron schedules, in-process messaging). **Driving** adapters, the ones that call into the use cases, sit in `scylla-core`: `grpc/` and `rest/`.

### Use case
A struct in `application/<feature>/mod.rs` (e.g. `PipelineUseCases`, `JobUseCases`) that holds `Arc<dyn Port>` fields and implements the `Run` stages of its commands and queries. It has no methods. Handlers run the actions through `Actions` (`grpc::adapter::run`).

### Entity
A domain object with an identity and mutable state (e.g. `Pipeline`, `Job`, `App`, `Organization`). Defined in `domain/<aggregate>.rs`.

### Value object
An immutable, validated wrapper in `domain/<aggregate>.rs` or `domain/<aggregate>/` (e.g. `PipelineName`, `NodeId`, `WorkingDir`, `JobStatus`). Built via a fallible constructor that enforces the invariant.

Each bounded string is a `Text<R>` from `domain/text.rs`. The marker `R` is a `Rule`: it gives the label, the maximum length and the checks. The maximum counts UTF-8 bytes. No kind accepts U+0000. A one-line kind (a name, a label, a key) also refuses every other control character. A free-form kind (a description, a script, an argument, a value) keeps them. A `SECRET` kind does not show its value in `Debug` or `Display`. A value read from the database goes through the same checks, so a row that breaks a rule fails when it is read.

### Repository
A port describing persistence for one aggregate (`PipelineRepository`, `JobRepository`, ...). The trait lives in `application/<feature>/repository.rs`; the PostgreSQL implementation lives in `scylla-db/src/postgres/` (one `Pg…Repository` per aggregate, queries via `sqlx::query!` / `query_as!`).

### Domain error
`DomainError` from `domain/errors.rs` with variants like `validation`, `business_rule`, `not_found`. Returned as `DomainResult<T>` from the kernel and mapped to gRPC statuses by the control plane's handlers.

### `JobEvent`
The lifecycle vocabulary a running agent reports: `JobStarted`, `NodeStarted`, `NodeCompleted`, `NodeFailed`, `NodeSkipped`, `JobCompleted`, `JobFailed`. Defined in the kernel because both binaries must agree on it: the agent emits it, the gRPC adapter maps it to and from its wire form, and the agent stream sends it as `RecordJobStatus`, which applies it to the `Job` aggregate.

### Kahn's algorithm
The topological-sort routine behind `DagPlan` (`domain/pipeline/dag.rs`). One implementation serves both sides: the control plane calls `drains_completely()` once to reject a pipeline containing a cycle, and an agent drives the same structure incrementally (`drain_ready` / `mark_completed` / `mark_terminal`) to decide what to launch next. Keeping it single is what stops the two from disagreeing about which nodes are runnable.

### Cargo features
One in the whole workspace. `test-utils` exposes the `test_support` builders (`scylla-core`) and seeders (`scylla-db`) to downstream test code. `scylla-domain`, `scylla-agent`, `scylla-proto`, `scylla-auth`, `scylla-extension`, `scylla-server` and `scylla-ce` have none.

### Extension point
A position in the action pipeline (`scylla-extension`). Every write is a command that `Actions::run` moves through `Authorize`, `Prepare` and `Persist`, every read a query that the same `run` moves through `Authorize` and `Fetch`; around each stage the `Hooks` registry runs `Policy`, `Gate`, `Around`, `Wrap`, `Listener` and `Observer`. An edition implements `Extension` and registers it on the `Server` builder (`.extension(&Arc::new(MeteredQuota))`); the core runs the hooks and never knows which edition built it. The Community Edition registers nothing. `scylla-extension` depends on `scylla-domain` only, so a private Enterprise build implements the hooks against a pinned git tag.

