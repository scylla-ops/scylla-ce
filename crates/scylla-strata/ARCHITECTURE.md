# scylla-strata

The CE backend on strata: every slice, every worker, the admin bootstrap and the webhook route. Strata is the only engine of CE. `crates/scylla-server` builds the edition of the binary (`Server::strata`, else `EditionHost::<ScyllaCeOnDb>`) and mounts every slice of `Slice::ALL`. The edition marks the slices that it serves (`EditionHost::new`); the server refuses to start when a slice is missing.

An old config with `[strata].slices` (or the env `SCYLLA_STRATA_SLICES`) still starts: the server ignores the value and writes one WARN line, "[strata].slices is ignored: every slice runs on strata".

## Writing a slice

Every CE slice is on strata. Use these rules for a new slice or a new use case. Pattern: see `src/projects.rs`, `src/secrets.rs`, `src/grpc/projects.rs`, `src/grpc/secrets.rs`.

1. One strata module per slice, `src/<slice>.rs`: `#[strata::module(section = "<slice>")] pub trait <Slice>: crate::Access`.
2. Components are the CE ports as trait objects: `#[component] type XRepo = Arc<dyn XRepository>;`. The server gives them with `Builder::provide_arc`. No twin traits, no newtypes.
3. One `#[usecase]` per use case. Its inputs are its parameters and its access is in its attribute (see "Use case parameters"). Then the reads, the checks and the writes, with their side effects (dispatch, registry, audit). The output is the domain value of the RPC, or `()` for a delete whose RPC returns nothing.
4. gRPC adapter. In the 12 services that CE puts behind its auth interceptor (User, Organization, Project, Pipeline, Trigger, Job, App, Secret, Agent, AgentAdmin, Grant, Role), the FIRST line of every RPC closure is `authed(&call).await?;` (before `parse(req)?`): CE refuses before it parses. Not in AuthService and AppAuthService (public in CE). Adapter details: `src/grpc/<slice>.rs`: `strata_tonic::service!` over `Grpc<A>`, the generated server in `super::service::<pkg>`, `let (a, b) = parse(req)?;` (the CE `Parse` impls) and the CE mappers. The response shape is the contract of the proto: the parity scenario (`binaries/scylla-parity`) checks it.
5. Tests `tests/<slice>.rs`: the use cases on stub ports, with the CE stubs (`scylla_core::test_support`, feature `test-utils`). An edition in the test file: `#[strata::edition(modules(scylla_strata::Access, scylla_strata::<Slice>))]`, then a derived edition that overrides `Access` (`Authenticator = DevTokens`, a `Permissions` stub). A flow on Postgres goes in `crates/scylla-server/tests/pg_edition.rs`, on `strata_shared` and `EditionHost::edition`.

## Use case parameters

The command and query structs of the Actions vocabulary are gone from CE: each use case of the 12 modules takes its inputs directly as parameters (the fields of the request, as `scylla_core::domain` types). Each parameter is a plain identifier. The use case declares its permission in its attribute, from its parameters: `#[usecase(write, access = Access::Requires(Permission::CreateProject(organization_id.clone())))]`. The access expression must clone or borrow a parameter (else E0382). When two use cases or a module method need the same access, put the access in one private function and call it from the attribute (`pipelines::run_access`, `triggers::manage_access`, `users::update_user_access`, `jobs::job_logs_access`). Keep one copy of each access. The `Parse` impl of the request gives the inputs as a tuple (`type Into = (A, B)`), in the order of the parameters, and the adapter writes `let (a, b) = parse(req)?;`. Keep the order of the parse checks and their gRPC codes and messages: check in the old order, return in the parameter order. Thus bind each input with `let` in the old order of the checks, then build the tuple in the order of the parameters. A request with two bad fields gives the error of the field that the old parse checked first. An error text names the wire field (`missing project_id`, `missing secret_id`).

The parameter order of a use case is a contract with EE: a named hook must name all inputs in that order. A hook on `M::uc` names each input by reference, in the order of the use case (else E0277). Thus do not add, remove or reorder a parameter of a hooked use case without a change in EE. A hook on a group (`M::writes`, `M::reads`) names no input: it reads the name of the use case and `self.access_of::<Access>()`.

The compiler checks the inputs of a named hook by their types, not by their names. A use case with two inputs of the same type (for example `change_password(current_password, new_password)`, two `Password`) also builds when a hook swaps them. Thus a hook on such a use case needs a test that shows which input it reads, not only the build.

The session of the call is not an input: a use case reads it from the context (`self.ctx().session()`), as `change_password`, `revoke_user_sessions` and `list_user_sessions` do. The client of a login also comes from the context.

The CE use cases that an EE hook wraps today (`ee-0.6.3`, `crates/scylla-ee-edition`):

- Named hook: `Grants::create_grant` (`#[before]`, `limits.rs`, the seat check).
- Group hooks (`#[after]`, `audit.rs`, no inputs): the writes of `Projects`, `Secrets`, `Pipelines`, `Apps`, `Roles`, `Grants`, `Organizations`, `Users`, `Auth`, `Triggers`, `JobControl` and `Agents`.

The CE tests also hook `Projects::create_project` (`organization_id, name, description`) and `Grants::create_grant` by name (`crates/scylla-server/tests/pg_edition.rs`).

## Service table

`Service::slice` (`src/host.rs`) maps each of the 14 gRPC services to its slice. `Built::mount` mounts the services of each slice that it gets, and starts the worker of each slice (`Worker::slice`). Do not add a second table.

## Fire audit

A fire runs the pipeline as the runner of the trigger (`run_as_runner`) and records its outcome (`record_as`) as module methods, so no hook sees them. They are not use cases: the run uses the access of `run_pipeline` (`pipelines::run_access`), and the record the access of `record_trigger_fire` (`triggers::manage_access`). The action names that the slot gets stay `run_pipeline_with_inputs` and `record_trigger_fire`. The fire gives each run to the `Triggers::Audit` slot (`FireAudit`). CE binds `NoFireAudit`. An edition overrides the slot to keep an audit row.

## Accepted log difference: no action span

The Actions engine is gone from CE. Thus CE writes no `action` span and no "persist stage ran" line. A log reader that keyed on them must read the strata lines, or the `audit_log` rows of the access model. This difference is accepted: no client sees it.

The old log (the Actions engine, with an observer on the persist stage):

```
2026-10-26T09:29:20.279373Z  INFO action{kind="command" action=01m5vrnz0000000000000000aa caller=user:01m5vrnttt2vbd3csy7bnj0h7g access=createProject resource=organization:01m5vrnzasfqgm5vgeqmdv2ay0}: scylla_ee: persist stage ran action=01m5vrnz0000000000000000aa caller=user:01m5vrnttt2vbd3csy7bnj0h7g access=createProject outcome="ok"
```

The new log (strata): the same call writes no line at `INFO`. The server writes the engine of each part at start:

```
2026-10-26T09:32:22.164991Z  INFO scylla_strata: the engine runs the worker worker=CronTriggers engine="strata"
2026-10-26T09:32:22.700035Z  INFO scylla_strata::host: strata serves the slice slice=projects edition="scylla_strata::ScyllaCeOnDb"
```
