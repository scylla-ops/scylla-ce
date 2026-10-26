# scylla-strata

The slices of the CE backend on strata. `crates/scylla-server` mounts the strata service of a slice instead of its Actions handler when `[strata].slices` (or `SCYLLA_STRATA_SLICES`) names it. The edition of the binary marks the slices that it serves (`EditionHost::new`); the server refuses to start with a switch for another slice.

## Porting a slice

Pattern: see `src/projects.rs`, `src/secrets.rs`, `src/grpc/projects.rs`, `src/grpc/secrets.rs`.

1. One strata module per slice, `src/<slice>.rs`: `#[strata::module(section = "<slice>")] pub trait <Slice>: crate::Access`.
2. Components are the CE ports as trait objects: `#[component] type XRepo = Arc<dyn XRepository>;`. The server gives them with `Builder::provide_arc`. No twin traits, no newtypes.
3. One `#[usecase]` per CE command or query. The parameter is the CE command or query struct (`scylla_core::application::<slice>::...`). The first line is `self.require_access(cmd.access()).await?;` (the `Describe` impl of CE, so the access check is the same as the Actions authorize stage). Then the logic of `Run<Prepare<C>>` and `Run<Persist<C>>` (or `Run<Fetch<Q>>`), in that order, with the same side effects (dispatch, registry, audit). The output is the CE committed/fetched type, or `()` for a delete whose RPC returns nothing.
4. gRPC adapter. In the 12 services that CE puts behind its auth interceptor (User, Organization, Project, Pipeline, Trigger, Job, App, Secret, Agent, AgentAdmin, Grant, Role), the FIRST line of every RPC closure is `authed(&call).await?;` (before `parse(req)?`): CE refuses before it parses. Not in AuthService and AppAuthService (public in CE). Adapter details: `src/grpc/<slice>.rs`: `strata_tonic::service!` over `Grpc<A>`, the generated server in `super::service::<pkg>`, `parse(req)?` (the CE `Parse` impls) and the CE mappers. The response shape must be the one of the CE handler (`crates/scylla-core/src/grpc/handlers/<slice>_handler.rs`): copy its mapping.
5. Tests `tests/<slice>.rs`: port the CE application tests of the slice (`crates/scylla-core/src/application/<slice>/tests.rs`) to the strata engine, with the CE stubs (`scylla_core::test_support`, feature `test-utils`). An edition in the test file: `#[strata::edition(modules(scylla_strata::Access, scylla_strata::<Slice>))]`, then a derived edition that overrides `Access` (`Authenticator = DevTokens`, a `Permissions` stub).
