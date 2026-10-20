# scylla-extension: the action pipeline

Every write in Scylla is a command that moves through three typed stages,
every read is a query that moves through two, and an extension attaches to
each stage. This crate holds the pipeline. It depends on `scylla-domain` and
nothing else: no access model, no database, no gRPC, no Cedar. A hook sees
`CallerContext`, `Access` (with its `Permission` values) and `DomainError`,
never a repository or a wire type. An Enterprise build compiles it from a
pinned tag.

Run `cargo test -p scylla-extension` for the in-crate checks. They drive the
engine with a self-contained aggregate (`src/tests.rs`), so a change to the
pipeline is proven here before it reaches a use case.

## Layout

```
src/
  action/        the event
    id.rs          ActionId: one per run
    command.rs     Describe: access(); Command: Staged, Committed; Query
    value.rs       Draft<T> (not stored), Deleted<T> (tombstone)
    envelope.rs    Envelope<C>: id, at, caller, access, command; one Arc
    phase.rs       Requested, Authorized, Prepared, Committed, Fetched
    erased.rs      Action: the erased view of a phase; Phase: its envelope
  stage.rs       StageKind, Stage, Authorize/Prepare/Persist, Fetch, Run<S>
  authz.rs       Access, Granted, Authorizer, AuthorizeStage
  hooks/         the extension seam
    position.rs    Policy, Gate, Around, Wrap, Listener, Observer
    next.rs        Next: rest of a typed chain; Proceed, Done: the erased one
    registry.rs    Hooks: registration and run()
    extension.rs   Extension: register(self: &Arc<Self>, &mut Hooks)
  path.rs        Path<K, R>: the chain after Authorize (Write, Read)
  actions.rs     Actions: run(runner, caller, action)
  tests.rs       the checks, on a fake aggregate
```

The core side lives in `scylla-core`: `application/actions.rs` adapts
`PermissionService` to `Authorizer`, `application/project/` is the reference
use case (`commands.rs`, `queries.rs`, the ports in `mod.rs`), and
`grpc/handlers/project_handler.rs` is the reference adapter.

## The event

One action is one event. It is born in `Requested<C>`, where `C` is the
command or the query. `Requested<C>` holds an `Arc<Envelope<C>>`: the action
id, the time, the caller, the access rule and the complete command. Every
later phase keeps the same envelope. All phases deref to it, so `caller()`,
`access()`, `command()` and `id()` are available at each step. No phase has a
public constructor. The only way to get a later phase is a transition method
on the earlier phase, and each transition consumes its input.

| phase           | new data                       | who produces it       |
|-----------------|--------------------------------|-----------------------|
| `Requested<C>`  | envelope                       | `Actions`             |
| `Authorized<C>` | nothing; `Granted` is consumed | the authorize stage   |
| `Prepared<C>`   | `C::Staged`                    | the use case          |
| `Committed<C>`  | `C::Committed`                 | the store             |
| `Fetched<Q>`    | `Q::Output`                    | the use case          |

`Granted` is a token with a private constructor and no data. Only
`AuthorizeStage` makes one, and `Requested::authorized` consumes it, so an
`Authorized<C>` is proof that the access check ran.

A command or a query declares its access rule through `Describe`, and its
payload types through `Command` or `Query`. Nothing else: which path it takes
follows from the trait.

`Access` has four values. The authorize stage applies them as follows:

| access                    | who passes                        | calls the `Authorizer`   |
|---------------------------|-----------------------------------|--------------------------|
| `Public`                  | every caller, `Anonymous` too     | no                       |
| `Authenticated`           | every caller except `Anonymous`   | no                       |
| `Requires(p)`             | a caller that has `p`             | one time, for `p`        |
| `RequiresAll(vec![p, q])` | a caller that has `p` and `q`     | for each, in order       |

`Authenticated` refuses `Anonymous` with `Forbidden`, the same error as the
Cedar check. For `RequiresAll`, the first refusal stops the check, and the
next permissions are not asked. Use `RequiresAll` when the action must refuse
on two permissions, for example a trigger update, which needs
`manageTriggers` and `runPipeline`. `Access::permissions()` gives the
permissions as a slice: empty for `Public` and `Authenticated`. For each
access, the hooks run in the same way: `Public` and `Authenticated` also go
through `Authorize`, and a `Granted` is minted for them without the
`Authorizer`.

```rust
pub struct CreateProject { pub organization_id: OrganizationId, pub name: ProjectName, ... }

impl Describe for CreateProject {
    fn access(&self) -> Access {
        Access::Requires(Permission::CreateProject(self.organization_id.clone()))
    }
}

impl Command for CreateProject {
    type Staged = Draft<NewProject>;
    type Committed = Project;
}

pub struct GetProject { pub id: ProjectId }

impl Describe for GetProject {
    fn access(&self) -> Access {
        Access::Requires(Permission::ReadProject(self.id.clone()))
    }
}

impl Query for GetProject {
    type Output = Project;
}
```

How the compiler picks the path (`path.rs`): coherence forbids "every
`Command`" and "every `Query`" as two blanket impls of one trait, because
nothing stops a type from being both. So the trait takes a marker parameter,
`Path<K, R>`, with one impl for `K = Write` over every `Command` and one for
`K = Read` over every `Query`. At a call `actions.run(runner, caller, action)`
the compiler tries both impls, keeps the one whose bounds hold, and infers `K`
from it. A wrong or missing `Run` impl on the runner is a compile error at the
call site, worded by `#[diagnostic::on_unimplemented]` on `Path`.

A query is not a command with an empty write. `Committed<T>` means "this is in
the store"; a read that went through a fake `Persist` would make that word
false. So a query has its own second stage and its own output phase, and
shares everything else: the envelope, the authorize stage, the hooks.

A `Draft<T>` is a value that is not in the store. A `Project` that comes out of
`Committed<CreateProject>` is in the store. For `DeleteProject`, `Staged` is
the loaded `Project` and `Committed` is `Deleted<Project>`, a tombstone with
the last state. The type tells you if the thing exists.

`Prepared<C>` has one way out: `commit(async |staged| ...)`. The closure gets
the staged value by move and must return a `C::Committed`. The store writes
inside the closure. A closure that does not write is a simulation, which is
what a dry-run `Wrap` does.

## The stages

Four stages, each a type with an `In` and an `Out` phase. A command takes the
first three, a query takes the first and the last:

- `Authorize<C>`: `Requested<C>` to `Authorized<C>`. `AuthorizeStage` runs it
  for every command and query; it applies the `Access` of the action, asks the
  `Authorizer` (in the core, the Cedar `PermissionService`) for each required
  permission and mints the `Granted`.
- `Prepare<C>`: `Authorized<C>` to `Prepared<C>`. The use case runs it. It
  builds the draft or loads the target. It reads, it does not write.
- `Persist<C>`: `Prepared<C>` to `Committed<C>`. The use case runs it too,
  against its repository port, by calling `commit` and writing inside the
  closure.
- `Fetch<Q>`: `Authorized<Q>` to `Fetched<Q>`. The use case runs it: it reads
  through the ports and returns the output.

`Run<S>` is the trait a stage implementation satisfies:
`async fn run(&self, input: S::In) -> DomainResult<S::Out>`. The signature is
fixed by `S`; the compiler, not the author, decides the phases.

`Actions::run(runner, caller, action)` is the one entry point. It mints the
envelope, opens the span and runs `Authorize`; then `Path::run` takes over
(`path.rs`): a `Command` runs prepare and persist and returns `C::Committed`,
a `Query` runs fetch and returns `Q::Output`. The `Path` impls carry the bound
the runner must satisfy, so a missing `Run` impl is a compile error at the
call site. The runner is the use case struct: one object that implements
the `Run` traits for its commands and queries. There is one `Actions` per
server, built in `init_services` with the permission service and the hooks,
and every action runs in one tracing span with its kind, id, caller, access
and resource; use cases carry no `#[instrument]` of their own. The `access`
field is `public`, `authenticated` or the permission keys joined by `+`; the
`resource` field is there only when a permission is required.

The access check is a stage and not a hook for two reasons. It runs with
zero hooks registered, because `Actions::run` calls it in code; a hook is
something a binary may or may not register. And it has an output,
`Authorized<C>`, that a `Gate` cannot produce: the use case's
`Run<Prepare<C>>` takes `Authorized<C>`, so its signature is the proof that
the check ran.

### The use case and the adapter

A use case struct (`ProjectUseCases`) holds the ports and implements the `Run`
traits. Each port is an `Arc<dyn Port>`, thus the struct, its `Run` impls and
its handler have no type parameters. It has no method of its own, no `Actions`
field, no access code and no hook code. The adapter (a gRPC handler) holds
`Arc<Actions>` and `Arc<ProjectUseCases>`, and each RPC is one call and its
response:

```rust
async fn create_project(&self, request: Request<CreateProjectRequest>)
    -> Result<Response<CreateProjectResponse>, Status> {
    let project = run(&self.actions, &*self.projects, request).await?;
    Ok(Response::new(CreateProjectResponse { project: Some(project_to_proto(&project)) }))
}

async fn list_projects(&self, request: Request<ListProjectsRequest>)
    -> Result<Response<ListProjectsResponse>, Status> {
    let page = run(&self.actions, &*self.projects, request).await?;
    Ok(Response::new(page.into()))
}
```

`grpc::adapter::run` (in `scylla-core`) takes the caller from the
interceptor, turns the request into its command or query through
`grpc::convert::Parse`, runs the engine and maps a `DomainError` to a `Status`.
An action that must keep the session of the call open (`ChangePassword`,
`RevokeUserSessions`) uses `grpc::adapter::run_in_session`: its request
implements `grpc::convert::ParseInSession`, which also gets the session that
the interceptor found (`caller_session`).
An action that opens a session (`Login`) uses
`grpc::adapter::run_public_with_client`: its request implements
`grpc::convert::ParseWithClient`, which also gets the client of the call
(`grpc::session_client(&request, trust_forwarded_headers)`: the user agent and
the IP address).
A service without the interceptor (sign-in, the app token exchange) uses
`grpc::adapter::run_public`: the same
steps, with `Anonymous` as the caller, so only a `Public` action passes. An
HTTP route uses `rest::adapter::run_public`: the route builds the command from
the path, the headers and the body, and maps the `DomainError` to its own
status.

The request types implement `Parse` in the aggregate's mapper
(`grpc/mappers/project_mapper.rs`), built from the small converters in
`grpc::convert`: `id` for a required id wrapper, `optional_id` for an optional
id wrapper (a wrapper with an empty value gives `INVALID_ARGUMENT`), `valid`
for a domain value built from a wire string, `proto_to_domain_pagination` for
a page. When each field is a required id, an optional id, the page or a field
copied as it is, the mapper uses `parse!` (`grpc/mappers/macros.rs`) and does
not write the impl. The same mapper turns a page into its response with
`From`: `page_response!` when the response is only the mapped items and the
page metadata. `access_to_proto!` makes the wire row of a `PrincipalAccess`,
because `UserAccess` and `AppAccess` have the same fields. An impl with other
logic stays written by hand. A handler never reads a request field itself.

### Adding a command or a query

A use case is one directory: `mod.rs` holds the struct with its ports,
`repository.rs` the port, `commands.rs` the writes, `queries.rs` the reads,
`tests.rs` the checks. Inside `commands.rs`, one block per command in the
order it runs: the struct with public fields, `impl Describe` with the
access, `impl Command` with the two payload types, `impl Run<Prepare<C>>`
(read through the port, build the domain value, `input.prepared(staged)`),
`impl Run<Persist<C>>`
(`input.commit(async |staged| self.repo.write(&staged).await).await`).
Inside `queries.rs`, one block per query: the struct, `impl Describe`,
`impl Query` with the output type, `impl Run<Fetch<Q>>` (read,
`input.fetched(output)`). A new action is one block in the right file, and
the outline of the file is the list of actions. A helper method of the use
case goes in `mod.rs`, not between two blocks.

`Staged` is a `Draft<T>` when `Prepare` builds or changes a domain value that
is not written yet: a new row, a changed row, a list of changed rows, or a
tuple of them. It is the loaded value for a delete, and a bare type only for
a parameter that is not a domain value (a count, a time, an id).

An aggregate with a second runner puts it in a subdirectory with the same
files: `mod.rs`, `commands.rs` (or `queries.rs`), `tests.rs`. For example
`app/token/`, `job/log/`, `trigger/fire/`, `trigger/webhook/` and
`agent/dispatch_use_case/`. A driver that holds `Actions` (a reaper, a
scheduler, a sweeper) has its own file, and its tests check only the driver;
the tests of the actions it sends are in the `tests.rs` of the use case.

A command derives `Debug` when each secret field is a redacting type
(`Password`, `AppSecret`, `ResetToken`): their `Debug` shows `[REDACTED]`. A command that
holds a secret as a raw `String` or as bytes has no `Debug`: `RevokeToken`,
`ValidateToken` (their `token`) and `IngestWebhook` (its headers).

A `tests.rs` gets its engine from
`test_support::authz::actions(permissions)`: the `PermissionAuthorizer` and
no hooks. Use `actions_with(permissions, hooks)` when the test registers
hooks. The stubs that more than one use case needs are in
`test_support::stubs` (compiled for tests only): `CountingPolicy`,
`StubHash`, `StubRegistry`, `StubRoles`, `StubGrants`, `StubJobs`,
`StubTails`, `ScopesByAgent`, `StubSessions`, `StubUsers`, `NoUsers`,
`OneUser`, `StubAccounts`, `OneProject`, `OnePipeline`, `EchoResolver`,
`dispatcher`, `empty_page` and `alice`. `StubUsers` keeps its users in memory
and gives `Conflict` for a username or an email of another user, as the store
does. `StubAccounts` keeps the sessions and the reset links of the users of a
`StubUsers`, and writes a user to that `StubUsers`. `StubHash::plain()` hashes
a password to `plain_hash(password)` and checks a password against it.
`StubRegistry` fails a test that sends a job to an
agent; `StubRegistry::accepting()` records each order. `StubRegistry::connect`
opens a stream of an agent. `StubJobs` checks and increments the version of a
job, as the Postgres store does, and `StubJobs::place` stores a job placed on
a stream. `dispatcher` makes a `DispatchUseCases` from these stubs. Keep a stub in the `tests.rs` of
the use case when
its behavior is specific to that use case, and give it a name that tells what
it does (`RowJobs`, `CountingRoles`), not the name of a shared stub.

A permission check that never refuses is not a gate, and a use case does not
ask the `PermissionService` a second time: each check writes an audit row. To
choose what a caller sees, a `Fetch` runner reads `VisibilityResolver::visible_scopes`,
which reads the grants and writes no audit row. `ListOrganizationProjects` uses
it to choose between every project of the organization (`listProjectsByOrganization`
there) and the projects that the caller can read (`readProject`).

## The hook positions

`Hooks::run` surrounds each stage, `Fetch` included, with six positions. They
run in this order:

```
Policy -> Gate<S> -> Around [ Wrap<S> [ Run ] ] -> Listener<S> -> Observer
 veto      veto      control   control            side effect     record
 every     one       every     one                one             every
```

Three of them are erased: `Policy`, `Around` and `Observer` are registered on
a `StageKind` and fire for every command that has a stage of that kind, also
for commands that do not exist yet. They receive `&dyn Action`: the action id,
`at()`, the caller, the `Access`, and `downcast_ref` to the typed phase.
Three of them are typed: `Gate<S>`, `Wrap<S>` and `Listener<S>` are registered
on one stage of one command, for example `Persist<DeleteProject>`, and receive
the exact phase type.

The rule to choose a position: decide first if the hook must be able to stop
the action (`Policy`, `Gate`), control how the work runs (`Around`, `Wrap`),
or only see the result (`Listener`, `Observer`). Then decide if the rule is
about one command (typed) or about a class of actions (erased). Then pick the
stage whose input or output carries the data the hook needs.

### Policy

An erased rule that the edition applies to a class of actions before a stage
runs. Signature: `enforce(&self, stage: StageKind, action: &dyn Action) ->
DomainResult<()>`.

Use it for: quotas, plan limits, rate limits, maintenance mode, feature
flags, kill switches. The decision reads `action.access().permissions()`,
`action.caller()`, and state the extension owns. Do not match on
`Access::Requires(..)`: that pattern does not see a permission inside
`RequiresAll`. A quota returns
`DomainError::quota_exceeded(..)`, which maps to `RESOURCE_EXHAUSTED`.

Do not use it for: a rule that needs one command's fields (use a `Gate`), a
side effect (use an `Observer`), anything that writes.

Which `StageKind`:

- `Authorize`: runs before the access check. Only for rules that must
  apply to unauthorized callers too, for example a rate limit or a
  maintenance mode. A quota here leaks quota state to callers who have no
  right to create.
- `Prepare`: runs after the access check and before the use case. The
  default position for a policy.
- `Persist`: runs after the use case and before the store. Use it when the
  rule needs the staged value, reached through `downcast_ref`.
- `Fetch`: runs after the access check and before the read. A rate limit
  on reads goes here, or on `Authorize` to cover reads and writes at once.

A `Policy` must be idempotent and must not write.

### Gate

A typed rule about one stage of one command. Signature:
`check(&self, input: &S::In) -> DomainResult<()>`.

Use it for: a rule that reads the fields of one command or one staged value.
"A project whose name starts with `prod-` cannot be deleted" reads
`Prepared<DeleteProject>::staged()`, so it is a `Gate<Persist<DeleteProject>>`.
"A trigger with a schedule needs the team plan" reads the command, so it is a
`Gate<Prepare<CreateTrigger>>`. A refusal is `DomainError::business_rule(..)`.

Do not use it for: a rule that applies to several commands (use a `Policy`,
or one `Gate` per command if the rules differ), a side effect, a change to
the input (it is a shared reference; a `Wrap` changes inputs).

Pick the stage by the data: the command alone is there at `Authorize`, the
staged value at `Persist`. A gate on `Persist` decides on the staged value as
read in `Prepare`; the version check makes sure the write goes through only if
the row is still in that state.

### Around

An erased hook that controls how a stage runs, for every command. Signature:
`around(&self, stage: StageKind, next: Proceed<'_>) -> DomainResult<Done>`.

Use it for: timing, a tracing span, a metric of failures, anything that must
surround every action of a stage kind. `next.action()` gives the input as
`&dyn Action`; `next.run().await` runs the rest of the chain and returns a
`Done`. The hook must return that `Done`; it cannot build one, so it cannot
skip the stage.

Do not use it for: anything that reads or changes the typed input or output
(use a `Wrap`), a decision (use a `Policy`; an `Around` that returns `Err`
without calling `next` is a veto in the wrong place).

Several `Around` on the same kind nest. The first registered is the
outermost. Every `Around` runs outside every `Wrap`.

### Wrap

A typed hook that controls how the stage's work runs for one command.
Signature:
`wrap(&self, input: S::In, next: Next<'_, S>) -> DomainResult<S::Out>`.

Use it for: a cache in front of a read, a dry-run mode, anything that needs
the typed input or output. It calls `next.run(input)` exactly once in normal
operation. A read cache is a `Wrap<Fetch<GetProject>>` that answers with
`input.fetched(cached)` on a hit and stores `fetched.output()` on a miss. A dry
run is a `Wrap<Persist<C>>` that never calls `next` and builds its output with
`input.commit(async |draft| Ok(draft.into_inner()))`.

Do not use it for: a business rule (a `Wrap` that returns `Err` to refuse is
a `Gate` in disguise), a side effect after success (use a `Listener`), a
retry (the input is moved into `next`; a retry must start from a new
`Actions::run`).

A `Wrap` may skip `next` only for a simulation, and then it must build the
output itself through the phase API. It cannot build an `Authorized<C>`,
because `Granted` has a private constructor, so it cannot skip the access
check. A simulation is not visible to `Listener` and `Observer`: they see a
success.

Several `Wrap` on the same stage nest. The first registered is the outermost.

### Listener

A typed side effect after one stage of one command succeeded. Signature:
`listen(&self, output: &S::Out)`. Returns nothing.

Use it for: a notification, an email, a webhook call, a cache invalidation
for one entity, a search index update. It reads the output and the caller
from the same value. It runs before `Actions::run` returns.

Do not use it for: a decision (too late, the work is done), a write that must
be consistent with the stage's write (put it in the `commit` closure or in a
database trigger; a listener runs outside the transaction and a crash
between the two loses it), a record for every command (use an `Observer`).

A `Listener` handles its own errors. It logs and returns; it has no way to
fail the action and must not panic.

### Observer

An erased record of what happened, on success and on failure. Signature:
`observe(&self, stage: StageKind, action: &dyn Action, outcome: Result<(),
&DomainError>)`. Returns nothing.

Use it for: an audit journal, metrics, usage counters, an event export or an
outbox writer, anything that must see every attempt.

On success, `action` is the output phase of the stage. On a `Policy` or
`Gate` veto, `action` is the input phase. On a `Run`, `Around` or `Wrap`
error, the input was consumed, so `action` is the envelope: id, time, caller
and access are there, and `downcast_ref` to a phase returns `None`. An
observer that counts must look at `outcome` first.

Do not use it for: a decision, a per-command effect with typed data (use a
`Listener`; a `downcast_ref` in an observer is acceptable for one field, such
as the organization of a deleted project, but a chain of downcasts means the
hook wants to be a `Listener`).

### Failure and order

- A `Policy` or a `Gate` that returns `Err` stops the action. Nothing to its
  right runs except the observers, no later stage runs, and `Actions::run`
  returns the error.
- A `Run`, an `Around` or a `Wrap` that returns `Err` stops the action. The
  `Listener` of that stage does not run; the observers run with the error.
- `Listener` runs only on success. `Observer` runs on both and cannot change
  the outcome.
- Within one position, hooks run in registration order. The order between
  extensions is the order of `Hooks::with` (or `Server::extension`) calls in
  the binary; `Feature::hooks` runs after the builder's `extension` calls.

### Decision table

| I want to                                        | position                      |
|--------------------------------------------------|-------------------------------|
| refuse a class of actions (quota, plan, flag)    | `Policy` on `Prepare`         |
| refuse before the access check (rate limit)      | `Policy` on `Authorize`       |
| refuse one action on its fields                  | `Gate<Prepare<C>>`            |
| refuse one action on its staged value            | `Gate<Persist<C>>`            |
| time, trace, count failures, for every action    | `Around` on each kind         |
| rate-limit reads and writes together             | `Policy` on `Authorize`       |
| cache one read                                   | `Wrap<Fetch<Q>>`              |
| simulate a write without writing                 | `Wrap<Persist<C>>`, skips next|
| notify after one action                          | `Listener<Persist<C>>`        |
| record every attempt, with its result            | `Observer` on each kind       |
| count what was stored                            | `Observer` on `Persist`       |

## Extensions

An extension is a type that implements `Extension` with
`register(self: &Arc<Self>, hooks: &mut Hooks)`. The receiver is the `Arc`,
so the same instance registers itself at several positions with
`self.clone()`, and the binary keeps the `Arc` to read the extension's state.

```rust
impl Extension for MeteredQuota {
    fn register(self: &Arc<Self>, hooks: &mut Hooks) {
        hooks
            .policy(StageKind::Prepare, self.clone())
            .observe(StageKind::Persist, self.clone());
    }
}
```

A binary registers it with `Server::extension(&Arc<E>)`, or a
`scylla_server::Feature` does it from `Feature::hooks`. The Community Edition
registers nothing.

### The reset sender

A reset link goes to the user through a port, not through a hook:
`scylla_core::application::PasswordResetSender`. The server has one sender.
The default, `LogPasswordResetSender`, writes the link in the server log. An
edition that sends mail implements the port and gives it to the builder:

```rust
struct SmtpResetSender { /* the mail client */ }

#[async_trait]
impl PasswordResetSender for SmtpResetSender {
    fn delivery(&self) -> PasswordResetDelivery {
        PasswordResetDelivery::Mail
    }

    async fn send(&self, message: &PasswordResetMessage) -> DomainResult<()> {
        // message.email, message.display_name, message.link, message.expires_at
    }
}

Server::new(config, db)
    .password_reset_sender(Arc::new(SmtpResetSender::new(mail)))
    .serve()
```

`delivery` must depend on the configuration of the sender only, never on the
account: `RequestPasswordReset` returns it for every email. `send` writes the
link only to its recipient: the link carries the token. An error of `send`
must not hold the email or the link, because the server can log it.
`RequestPasswordReset` stores the link and calls `send` in a task after the
answer, and logs a failure at WARN. `SendPasswordReset` calls it before the
answer, so the administrator sees the error. The link starts with
`[server].public_url`.

## Building blocks for an edition

An edition outside this repository uses these public items. It does not write
its own SQL on a table of this repository. `crates/scylla-db/tests/edition_api.rs`
is a crate of its own: it uses only the public API, so a change that hides one of
these items fails there.

- `scylla_auth::authz::check_grantable(roles, grants, delegator, grantee, role,
  scope, organization)`: the check of `CreateGrant` (the role, its scope kind, an
  agent role for a user, the escalation rule). `roles` comes from
  `RoleRepository::list_all`, `grants` from `GrantRepository::list_all`, and
  `organization` is the organization of `scope`. An organization role of another
  organization is an unknown role.
- `scylla_auth::authz::grantable_role(roles, grantee, role, scope, organization)`
  gives the `&Role`: `check_grantable` without the escalation rule.
  `ensure_no_escalation` is the escalation rule alone.
- `scylla_core::application::grant::CreateGrant` has public fields (`principal`,
  `role`, `scope`). A `Gate<Prepare<CreateGrant>>` reads them with
  `input.command()`. Register it with `hooks.gate::<Prepare<CreateGrant>>(..)`.
  It runs after the access check and before the checks of the use case, thus
  before `check_grantable`. The gate and the write are not in one transaction:
  two grants at the same time can both pass a count.
- `OrganizationRepository::list_principals`, `count_members` and `has_member`
  (`scylla_db::PgOrganizationRepository`): the members of an organization are
  the users with a grant on it or on one of its projects. An app is not a member.
- `scylla_db::postgres::signup::provision_user_in(&mut tx, &user, &grants)`
  writes a user and its grants in a transaction that the edition owns: the
  edition writes its own rows in the same transaction, then commits. Build the
  values with `NewAccount::without_organization(user).with_grant(role, scope)`. A
  taken username or email is `Conflict`. `PgSignupRepository::provision_user` is
  the same write in a transaction of its own.

## Versions

A row that `Prepare` reads and `Persist` writes carries a `version`. The store
writes an update or a delete only if the stored version is the staged one and
increments it on an update. If the row changed between the read and the
write, the write fails with `DomainError::Stale` (gRPC `ABORTED`) and nothing
is written. `DomainError::Conflict` (gRPC `ALREADY_EXISTS`) is only for a value
that already exists.
A write that is not an edit writes only its own columns, and it does not
check or change the version. For example, `RecordTriggerFire` writes only the
last fire of a trigger, and the cron passes write only its next fire time.
Thus a fire or a cron tick does not make an edit fail with `Stale`.
The caller starts again with a new `Actions::run`; a `Wrap<Persist<C>>` cannot
retry, because its input is the stale staged value. A `Gate<Persist<C>>` that
decided on the staged value is therefore safe: the write goes through only if
the row is still in the state the gate saw.

Projects, pipelines, triggers, organizations, users and jobs carry a version.
Each Postgres adapter gives the result of its write to `written`
(`scylla-db/src/postgres/version.rs`) and does not add its own helper. When
the write changed no row, `written` reads the row: if the row is there, the
error is `Stale`, else `NotFound`.

The placement of a job is not an edit, but it changes the version:
`JobRepository::claim_next` and `release` write only the agent and the stream
of the job, and they increment the version. Thus a cancel that read the job
before a placement gives `Stale`, and it cannot miss the agent that got the
job. A pass over jobs (`ReapOrphanedJobs`, `ReconcileAgentJobs`) skips a job
that gives `Stale`: a report of the agent came first.

## Limits and follow-ups

- The project, organization, user, secret, pipeline, trigger, app, agent, job,
  job log, grant and role use cases are on the pipeline. The session
  (`Login`, `ValidateToken`, `RevokeToken`), `IssueAppToken` and
  `IngestWebhook` are on it too, as `Public` actions. The writes of the server
  drivers and of the agent stream are on it too. Every use case is on the
  pipeline.
- A `Public` action runs as `Anonymous`, so a `Policy` on `Authorize` sees
  every sign-in and webhook delivery. The check that the use case does
  itself (the password, the app secret, the webhook signature) is in
  `Prepare`, before the write. The `Debug` rule
  of a command with a secret is in "Adding a command or a query".
- `Login` stages its session with `auth::new_session` and attaches the client
  of the call with `Session::with_client`. It verifies the
  password before it reads `is_active`. An unknown account and a wrong
  password give the same `Unauthorized("Invalid credentials")`. Only a caller
  with the right password gets "User account is inactive".
- The Enterprise sign-up extension uses `new_session`, `signup::NewAccount`
  and `SignupRepository`. Its handler can record the client of the call as
  `Login` does: `new_session(id).with_client(session_client(&request,
  ctx.trust_forwarded_headers))`, where `ctx` is the `scylla_server::Feature`
  `Context`. The Community Edition does not use `NewAccount` and
  `SignupRepository`, but they stay public for that extension. Its `commit`
  closure writes the account, then stores the session. `NewAccount::new`
  makes an account with its own organization, which `provision_account`
  writes. `NewAccount::without_organization(user)` makes a user and its
  grants only (`NewAccount<NoOrganization>`), which `provision_user` writes in
  one transaction. `with_grant` works for both. The default
  `provision_user` of the trait refuses with `Internal`, so a store that
  existed before it still compiles; `PgSignupRepository` writes the account.
- The reset links have their own runner, `PasswordResetUseCases`
  (`application/user/reset/`). `RequestPasswordReset` and `ResetPassword` are
  `Public`. `RequestPasswordReset` stages nothing for an unknown email or an
  inactive account. For an active account, its `commit` closure starts a task
  (`issue_and_deliver`) that stores the link with the 60-second rule and then
  delivers it. Thus the answer, its time and its errors are the same for each
  email, and a `Listener` on its `Persist` runs before the link is stored.
  `SendPasswordReset` asks for `UpdateUser` on the user. The writes that touch
  a user and what it signs in with (its sessions, its reset links) go through
  `AccountRepository` (`application/user/account.rs`), one transaction for
  each write. `ResetPassword` checks the link in `Prepare` and again in the
  write: a link that a concurrent call used gives the same error.
- `GetMe`, `ChangePassword` and `DeleteAccount` are `Authenticated`, and their
  target is the caller: `Prepare` (or `Fetch`) refuses a caller that is not a
  user (`application::actions::user_only`). `UpdateUser` with an email is
  `RequiresAll` of `UpdateUser` on the user and `CreateUser`, so the self rule
  of the Cedar policies does not let a user change its own email.
  `SetUserActive` and `RevokeUserSessions` ask for `UpdateUser` on the user;
  `SetUserActive` refuses the caller's own id in `Prepare`.
- `ListUserSessions` asks for `ReadUser` on the user and `RevokeUserSession`
  for `UpdateUser`; the self rule of the Cedar policies gives both to the user
  for its own account. Both read the sessions through `AccountRepository`
  (`list_sessions`, `revoke_session`), as `UserSession` rows without the token.
  `ListUserSessions` is a `ParseInSession` request: its `Fetch` marks the
  session of the call `current`. `RevokeUserSession` stages the session that
  `Prepare` found among the live sessions of the user, and its `Committed` is
  `Deleted<UserSession>`. An unknown id, a session of another user and an
  expired session give one `NotFound`, in `Prepare` or, after a concurrent
  revoke, in the write. It does not spare the session of the call.
- `ListUserAccess` and `ListAppAccess` return `PrincipalAccess` rows
  (`application/grant/access.rs`): each grant of the principal with the names
  of its organization, project and role. The two ports
  (`AccountRepository::list_access`, `AppRepository::list_access`) use one
  query on a `Principal` (`grants::list_access` in `scylla-db`).
  `ListUserAccess` asks for `ReadUser` and reads the user first, thus an
  unknown user gives `NotFound`. `ListAppAccess` asks for `ReadApp` and reads
  only the grants, thus an unknown App gives an empty list.
- `ListOrganizationJobs` with `agent_id` keeps its permission
  (`ListJobsByOrganization`) and lists only the jobs of the organization that
  the server placed on that agent (`jobs.agent_app_id`), at each state. The
  order and the pages do not change.
- `ValidateToken` is a query, and its `Fetch` only reads: an expired
  session gives `false` and stays in the store. A failed read also gives
  `false`. `PurgeExpiredSessions` deletes the expired sessions: it is a pass
  that `SessionSweeper` (`session-sweeper`) sends one time each hour.
- `IngestWebhook` gives `NotFound` for an unknown, disabled or non-webhook
  trigger and `Unauthorized` for a missing or wrong signature. A fire that
  fails with `Validation` (a payload value that a run cannot use) stays
  `Validation`. Every other failure becomes `Internal`. Thus the route answers
  404, 401, 422 or 500.
- The `AuthInterceptor` stays outside the pipeline. It is not an action: it
  makes the caller that the actions get. It and
  `ValidateToken` use the same session rule, `auth::look_up_session`. A
  failed read is `INTERNAL` in the interceptor and `false` in
  `ValidateToken`. The interceptor also accepts an app token. Its only write
  is `auth::record_activity`: for a live session whose `last_active_at` is
  `session::ACTIVITY_INTERVAL` (five minutes) old or older, one conditional
  `SessionRepository::touch`. A failed write is a WARN, not an error of the
  call. `ValidateToken` does not move the activity. The default `touch` of the
  trait refuses with `Internal`, so a store that existed before it still
  compiles; `PgSessionRepository` writes.
- The agent stream sends each write through `Actions`, as the agent's own
  token: `RecordJobStatus` for each status and `AppendJobLogs` for each batch
  of lines (the `WriteJobStatus` and `AppendJobLog` checks are the authorize
  stage), and `TouchAgent` and `RecordAgentHost` for the heartbeat and the host
  report. No permission gets to the agent's own App, because its grant is on
  an organization or a project. Thus `TouchAgent`, `RecordAgentHost`,
  `ReleaseAgentJobs` and `ReconcileAgentJobs` are `Authenticated`, the target
  is the caller, and `Prepare` refuses a caller that is not an App
  (`application::actions::app_only`). The stream sends `TouchAgent` before it
  registers the connection: if the action fails, the stream is refused with
  its status and the agent is not registered. The registration gives the
  stream its id and wakes the dispatcher for the agent. After each hello the
  stream sends `ReconcileAgentJobs` with the jobs that the agent runs: a
  running job of the agent that the list omits becomes orphaned, and a listed
  job that does not run on the agent gets a cancel. The agent sends its hello
  after the frames that the earlier stream did not take, so a job that ended
  while the agent was away reports its end first. When the stream ends, it
  sends `ReleaseAgentJobs` with its id.
- A status or a line counts only from the agent that the job is placed on,
  until the job ends (`Job::ensure_live_on`, a `BusinessRule`). The stream
  logs a refused status or batch at `DEBUG` and reads on. The stream reads
  the frames that are ready together (`ready_chunks`): the lines between two
  other frames make one `AppendJobLogs` for each job, so one check and one
  insert serve many lines, and a status never passes a line sent before it.
  The `commit` closure of `RecordJobStatus` opens the live tail at
  `JobStarted` and closes it when the job ends, and the `commit` closure of
  `AppendJobLogs` pushes the stored lines to the tail. The time of a status
  is the time of the agent, kept between the creation of the job and now.
  The read loop and the registry stay outside the pipeline.
- `TailJobLogs` is a query: its `Fetch` returns the stream, so hooks see the
  subscription open, not each line. `JobLogLiveStream` is `Sync` for that
  reason, because a query output is.
- `ListJobLogs` with a node is `RequiresAll` of `ReadJobLogs` and `ReadJob`.
  Its `Fetch` reads the job through the job port and applies
  `Job::logs_readable_for`: a node that is not readable yet gives an empty
  page. `TailJobLogs` with a node applies the same rule: it replays no stored
  line for that node.
- Each write of a server driver goes through `Actions`, as a sealed
  `CallerContext::Service`, with one identity for each driver: `JobReaper`
  (`job-reaper`) sends `ReapOrphanedJobs`, `PendingJobScheduler`
  (`job-dispatcher`) sends `DispatchPendingJobs`, `TriggerCronScheduler`
  (`cron-scheduler`) sends `ScheduleCronTriggers` and `ClaimDueTriggers`,
  `TriggerFirer` (`trigger-firer`) sends `ResolveTriggerRun` and
  `RecordTriggerFire`, and `SessionSweeper` (`session-sweeper`) sends
  `PurgeExpiredSessions`. The loops stay outside the pipeline: they are not
  requests.
- A pass over many rows (`ReapOrphanedJobs`, `DispatchPendingJobs`,
  `ScheduleCronTriggers`, `ClaimDueTriggers`, `PurgeExpiredSessions`) has no
  resource for Cedar. Thus it is `Authenticated`, and `Prepare` refuses a
  caller that is not a service (`application::actions::service_only`). The
  read `ResolveTriggerRun` does the same in its `Fetch`. The guards on the
  kind of caller are together in `application/actions.rs`: `service_only`,
  `app_only`, `user_only` (an action on the caller's own account) and
  `user_or_app` (the origin of a `RunPipeline`). A pass that
  runs each 15 or 30 seconds does not write an audit row each time. A write
  on one row asks for the permission of that row, as the bootstrap does:
  `RecordTriggerFire` asks for `ManageTrigger`, and the `service` rule of the
  Cedar policies permits it. That check writes one audit row for each fire.
- A job runs the nodes it was created with (`Job::nodes`); a dispatch never
  reads the pipeline again. `RunPipeline` and `RunPipelineWithInputs` build
  the job in `Prepare` and assemble its dispatch there (`assemble_dispatch`):
  a secret that does not resolve, or a job larger than `MAX_DISPATCH_BYTES`,
  stores nothing. The `commit` closure stores the job and wakes the
  dispatcher; a run does not place its job.
- `DispatchPendingJobs` is the only action that places a job. A wake names the
  agents of the pass, or all of them. For each connected agent that has no job
  that has not ended, it reads the grants of the agent
  (`VisibilityResolver::visible_scopes` with `executeJob`) and claims the
  oldest pending job that they cover on the open stream of the agent
  (`JobRepository::claim_next`, one statement). Thus a pass asks no permission
  and writes no audit row, and one job goes to one agent. A job that no longer
  assembles fails, and the agent gets the next one. A job that the stream does
  not take goes back to the pool, and the registry closes that stream, so the
  next pass does not try it again. A run, a released job, the end of a job, a
  new stream, a deleted App and a grant to an App wake the dispatcher; the
  pass also runs each 30 seconds.
- A placed job names the agent stream that took it (`jobs.stream_id`, a ULID
  for each registration, so a stream of an earlier process never matches a
  new one). The dispatcher sends the job only on that stream. A release names
  one stream and returns only the jobs of that stream that have not started
  (`DispatchUseCases::release`, which wakes the dispatcher). Each release
  names a stream that is gone: `ReleaseAgentJobs` when the stream ends, a pass
  whose order the stream did not take, and `ReapOrphanedJobs`. Thus a release
  cannot take a job from a newer stream of the same agent, and no release
  depends on the order of the steps of two streams.
- `CancelJob` asks for `UpdateJob` on the job. `CancelJob`, `DeleteJob`,
  `ReapOrphanedJobs` and `ReconcileAgentJobs` end a job on the server, then
  stop it on its agent with `DispatchUseCases::stop`: a `CancelJob` order and
  the close of the live tail. After a `CancelJob` order, the reference agent
  sends no more frames about the job. Thus a deleted job gets no report that
  the access check refuses. `DeletePipeline`, `DeleteProject` and
  `DeleteOrganization` put their delete in `DispatchUseCases::recall`: it
  reads the live jobs of the scope, does the delete, and stops those jobs
  only when the delete succeeded.
- `ReapOrphanedJobs` looks at the running jobs whose agent is not connected
  and was not seen for `ORPHAN_GRACE` (60 seconds): they become orphaned. It
  also releases each stream that holds a job that has not started and is not
  open, for example a stream of the process before a restart. It reads the
  open streams after the jobs: a stream opens before it can hold a job, so a
  stream that the pass finds closed is gone for good.
- `DeleteApp`, `SetAppActive` and `CreateAppSecret` read the App in `Prepare`.
  An unknown App gives `NotFound`, and the trigger-runner App gives
  `BusinessRule` (`App::ensure_user_managed`). `App::create` refuses the name
  `trigger-runner` with `Validation`, so `CreateApp` and `CreateAgent` cannot
  make an App with that name. When the request has no secret label,
  `CreateApp` and `CreateAgent` get the label of the first secret from
  `default_secret_label` (`scylla-domain`): the App name, cut to the label
  rules, else `default`. `AgentAdminService.DeleteAgent` sends
  `DeleteApp`. Each close of an agent stream comes after its write:
  `DeleteApp`, `SetAppActive`, `RevokeAppSecret`, `SetAppSecretEnabled`,
  `RevokeGrant`, `RevokeAllAccess`, and `DeleteOrganization` for each App of
  the organization. The delete of an App returns its jobs that have not
  started to the pool (`ON DELETE SET NULL`), so `DeleteApp` wakes the
  dispatcher.
- `ListGrantableRoles` without an organization and `GetMyPermissions` are
  `Authenticated` queries: the platform roles are visible to everyone, and a
  caller reads its own grants. With an organization, `ListGrantableRoles`
  requires `ReadOrganization` there and adds the roles of that organization.
  `ListAuthzVocabulary` is `Authenticated`: the catalog is compiled in. The gRPC
  interceptor refuses a call without a token, so `Anonymous` does not get to
  them from an RPC. `GetMyPermissions` refuses a service caller in its `Fetch`
  runner: a service holds no grants, and an empty list would read as "no
  permissions".
- `CreateGrant` builds the grant in `Prepare` and checks there, in this order,
  the organization of the scope, `check_grantable` (the role, its scope kind,
  an agent role for a user, the escalation rule) and the admission of the
  grantee: an app only in its own organization, a user on a project only after
  a grant on its organization. A repeated grant returns the stored grant. The
  bootstrap admin grant goes through `Actions` as the bootstrap service.
- The bootstrap (`application/bootstrap.rs`) is a driver. It sends
  `CreateUser`, then on a `Conflict` it finds the admin account by the
  configured email (`GetUserByEmail`). If no account has that email, it finds
  the account by the configured username (`GetUserByUsername`) and sets the
  email with `UpdateUserEmail` only when that account has no email. Any other
  case is a `BootstrapError`, and the account gets no grant. No RPC sends
  these three actions.
- `RevokeGrant`, `RevokeAllAccess` and `DeleteUser` call `ensure_owner_remains`
  in `Prepare` with the grants that they remove. `RevokeGrant` of the last
  grant of a user on an organization stages `whole_organization`, and
  `Persist` then calls `revoke_all`, as `RevokeAllAccess` does.
- `RoleUseCases` lives in `scylla-core` (`application/role/`), not in
  `scylla-auth`: the `Run` impls need the struct in the crate that names the
  commands. `scylla-auth` keeps `Role`, `RoleRepository` and
  `validate_role_permissions`.
- `DeleteSecret` asks for its permission on the secret, not on the project.
  The access model finds the project of the secret in `Authorize`
  (`ResourceRef::Secret`, one join in `PgAuthzEntityProvider`). An unknown
  resource of any kind is under System only: without a System grant the caller
  gets `Forbidden`; with a System grant, the use case gives `NotFound`. Thus
  each use case must read the row that an id names. An action whose
  permission is on a parent that only the loaded row knows can use the same
  method.
- `GetTrigger`, `UpdateTrigger`, `SetTriggerEnabled`, `DeleteTrigger` and
  `FireTriggerNow` use the same method: they ask for their permission on the
  trigger (`ResourceRef::Trigger`, one join to the pipeline). `ManageTrigger`
  and `RunTriggerPipeline` have the keys `manageTriggers` and `runPipeline`, so
  the same roles give them. They are not in the permission catalog, because a
  key is there one time only. `CreateTrigger` and `ListPipelineTriggers` keep
  `ManageTriggers` on the pipeline.
- `RevokeAppSecret`, `SetAppSecretEnabled` and `SetAppSecretLabel` use the
  same method: they ask for `ManageAppSecret` on the secret
  (`ResourceRef::AppSecret`, one join to the app and its organization). It has
  the key `deleteApp`, so the same roles give it, and it is not in the
  permission catalog. `CreateAppSecret` and `ListAppSecrets` keep their
  permission on the app.
- `RevokeGrant` uses the same method: it asks for `RevokeGrant` on the grant
  (`ResourceRef::Grant`, one read of the grant scope, with a join to the
  organization of a project scope). The access model then applies the
  manage-grants action of the scope kind: `manageProjectGrants`,
  `manageOrgGrants` or `manageSystemGrants`. The key of `RevokeGrant` is
  `manageSystemGrants`, the key of an unknown grant, and it is not in the
  permission catalog. An unknown grant is under System: without a
  `manageSystemGrants` grant the caller gets `Forbidden`; with one, the call
  succeeds and changes nothing, as before. `CreateGrant`, `RevokeAllAccess`
  and `ListGrants` keep their permission on the scope.
- `GetRole`, `UpdateRole` and `DeleteRole` use the same method: they ask for
  `ManageRole` on the role (`ResourceRef::Role`, one read of its owner). The
  access model applies `manageOrgRoles` for a role that an organization owns,
  and `manageRoles` for a platform role or an unknown role. `CreateRole` and
  `ListRoles` ask for `ManageOrgRoles` on the organization they name, or for
  `ManageRoles` without one. `CreateRole` and `UpdateRole` call
  `ensure_no_escalation`, as `CreateGrant` does, so a role holds only
  permissions that its author holds.
- The Cedar policy set validates each role template alone. A role whose
  template does not validate is skipped with an error log: its grants give
  nothing, and the other roles keep working.
- `CreateTrigger` is `RequiresAll` of `ManageTriggers` and `RunPipeline` on
  the pipeline, and `UpdateTrigger` is `RequiresAll` of `ManageTrigger` and
  `RunTriggerPipeline` on the trigger: managing triggers must not give run
  rights. Both checks are in `Authorize`, so a `Policy` on `Prepare` runs
  after them. The runner app of the organization is provisioned in `Persist`,
  next to the trigger write. The store finds the runner by its kind
  (`AppKind::TriggerRunner`), not by its name, and the runner has no secret.
- `FireTriggerNow` fires in its `commit` closure through the `TriggerFiring`
  port, as a scheduled fire does. `IngestWebhook` records the delivery and
  fires in its `commit` closure. If the fire fails, it removes the delivery
  (`TriggerDeliveryRepository::forget`), so a retry with the same delivery id
  fires. Without a delivery id, the dedupe key is the verified digest in
  lowercase hex (`verified_digest`), so all the forms of one signature are one
  delivery. `TriggerFirer` is that port for the cron
  scheduler, the webhook ingress and `FireTriggerNow`. It reads the trigger
  and the trigger-runner App of the organization with `ResolveTriggerRun`,
  which refuses a disabled trigger. Then it sends `RunPipelineWithInputs` as
  that App (one `RunPipeline` check), then sends `RecordTriggerFire`,
  best-effort. A runner App that is not found is a failed run: the fire
  records it. `FireTriggerNow` stages only the id, because the fire reads the
  trigger. `TriggerFirer` holds `Actions`, thus it is a driver and not a use
  case: a use case gets to it through the port.
- `RunPipelineWithInputs` is the fire path of a run: the origin is the trigger
  and the inputs come from the trigger, so no RPC sends it. `RunPipeline` is
  the RPC path. Each input is an `EnvKey` and an `EnvValue`. A payload value
  that is not a valid `EnvValue` (it has a NUL, or it is more than 64 KiB)
  stops the fire with a validation error before the job is stored.
- `RecordTriggerFire` carries only the trigger id and the status, and writes
  only the last fire (`TriggerRepository::record_fire`); see "Versions".
- A `scylla_server::Feature` gets `actions` in its `Context`. An installed
  service authorizes through `Actions::run`, as the core does, so its
  actions go through the hooks. `permissions` stays in the `Context` until
  the Enterprise features use `actions`; `visibility` serves a scope in a
  `Fetch`. `trust_forwarded_headers` is `[server].trust_forwarded_headers`,
  for `session_client`.
- No use case rebuilds the Cedar policy set. A database trigger increases
  `authz_version` on each change to `grants`, `roles` or `role_permissions`,
  and each check rebuilds the set when the version changed. A write that
  changes a grant, a feature included, needs no extra call.
- The Cedar entity provider loads the resource's ancestors during `Authorize`;
  a missing resource surfaces there, before the permission decision.
