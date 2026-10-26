# scylla-strata

The CE backend on strata: every slice, every worker, the admin bootstrap and the webhook route. Strata is the only engine of CE. `crates/scylla-server` builds the edition of the binary (`Server::strata`, else `EditionHost::<ScyllaCeOnDb>`) and mounts every slice of `Slice::ALL`. The edition marks the slices that it serves (`EditionHost::new`); the server refuses to start when a slice is missing.

An old config with `[strata].slices` (or the env `SCYLLA_STRATA_SLICES`) still starts: the server ignores the value and writes one WARN line, "[strata].slices is ignored: every slice runs on strata".

## Writing a slice

Every CE slice is on strata. Use these rules for a new slice or a new use case. Pattern: see `src/projects.rs`, `src/secrets.rs`, `src/grpc/projects.rs`, `src/grpc/secrets.rs`.

1. One strata module per slice, `src/<slice>.rs`: `#[strata::module(section = "<slice>")] pub trait <Slice>: crate::Access`.
2. Components are the CE ports as trait objects: `#[component] type XRepo = Arc<dyn XRepository>;`. The server gives them with `Builder::provide_arc`. No twin traits, no newtypes.
3. One `#[usecase]` per CE command or query. The parameter is the CE command or query struct (`scylla_core::application::<slice>::...`). The first line is `self.require_access(cmd.access()).await?;` (the `Describe` impl of the CE type). Then the reads, the checks and the writes, with their side effects (dispatch, registry, audit). The output is the domain value of the RPC, or `()` for a delete whose RPC returns nothing.
4. gRPC adapter. In the 12 services that CE puts behind its auth interceptor (User, Organization, Project, Pipeline, Trigger, Job, App, Secret, Agent, AgentAdmin, Grant, Role), the FIRST line of every RPC closure is `authed(&call).await?;` (before `parse(req)?`): CE refuses before it parses. Not in AuthService and AppAuthService (public in CE). Adapter details: `src/grpc/<slice>.rs`: `strata_tonic::service!` over `Grpc<A>`, the generated server in `super::service::<pkg>`, `parse(req)?` (the CE `Parse` impls) and the CE mappers. The response shape is the contract of the proto: the parity scenario (`binaries/scylla-parity`) checks it.
5. Tests `tests/<slice>.rs`: the use cases on stub ports, with the CE stubs (`scylla_core::test_support`, feature `test-utils`). An edition in the test file: `#[strata::edition(modules(scylla_strata::Access, scylla_strata::<Slice>))]`, then a derived edition that overrides `Access` (`Authenticator = DevTokens`, a `Permissions` stub). A flow on Postgres goes in `crates/scylla-server/tests/pg_edition.rs`, on `strata_shared` and `EditionHost::edition`.

## Service table

`Service::slice` (`src/host.rs`) maps each of the 14 gRPC services to its slice. `Built::mount` mounts the services of each slice that it gets, and starts the worker of each slice (`Worker::slice`). Do not add a second table.

## Fire audit

A fire runs `run_pipeline_with_inputs` and `record_trigger_fire` as module methods, so no hook sees them. The fire gives each run to the `Triggers::Audit` slot (`FireAudit`). CE binds `NoFireAudit`. An edition overrides the slot to keep an audit row.

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
