---
name: change-extension-point
description: Change the contract between the Scylla core and its editions in scylla-extension. This means a new hook on ResourceGuard (for example after_create for an audit event), a new value in a context, a new trait, or several guards. Use when an edition must act on something that the core does not show yet. Never put edition code in the core instead. To add an EE gRPC service, work in the Enterprise repository: the contract already has ServiceProvider.
---

# Change an extension point

The contract is `scylla-extension`. The core calls its traits and never names an implementation. Each edition gives its implementations in an `Edition` value (`scylla-core/src/app.rs`).

The contract has three traits today:

- `ResourceGuard`: refuses or follows a create or a delete, in the transaction of the core.
- `CapabilityProvider`: the edition name and its optional features.
- `ServiceProvider`: the gRPC services that the edition adds next to the core services.

`Edition` has one field for each trait. When an edition has more than one implementation for a trait, it puts them together in one value: one value lists all its capabilities, one value adds all its services, and `GuardChain` (see "Several guards") calls all guards.

The Enterprise edition is in a separate repository. It gets a change of this crate only when it moves to a new tag of this repository. Do not change the Enterprise repository from this repository.

## Rules that never change

- Use general words: resource, kind, owner, guard, capability, service, rejection. Never use an edition word (quota, plan, tenant, enterprise). `scripts/check-structure.sh` stops these words in the public crates.
- The core holds each implementation as `Arc<dyn Trait>` in `Edition`, and gives `&dyn Trait` to the service functions. Do not make the core generic over a trait. With generics, the core compiles one time for each edition.
- A trait is `Send + Sync + 'static`. An async trait uses `#[async_trait]`, which `scylla_extension::async_trait` re-exports.
- A hook that runs in a core write takes `&mut Transaction<'_, Postgres>` from the caller. It gets `&mut`, so it cannot commit. The core commits.
- A new method on an existing trait has a default body that does nothing (`Ok(())`). Then `AllowAll` and each existing implementation compile with no change.
- Each new trait has a no-op implementation in `scylla-extension`. The Community edition uses it, like `AllowAll`, `CommunityCapabilities` and `NoExtraServices`.
- A context struct has `#[derive(Clone, Copy, Debug)]`, `#[non_exhaustive]`, `pub` fields and a `new()` function. With `#[non_exhaustive]`, other crates must use `new()`, so nobody can forget a new field.
- The type of each context field is in `scylla-domain` or in `std`. `scylla-extension` depends only on `scylla-domain`, `sqlx`, `async-trait`, `thiserror` and `tonic`. It uses `tonic` only for the `RoutesBuilder` of `ServiceProvider`.
- A change in `scylla-extension` recompiles the core, the test kit, the Community edition, and the Enterprise edition at its next tag. Keep the contract small.

## A new hook on ResourceGuard

Use a hook for each action of an edition before or after a core write: a refusal, a counter, an audit event, a notification. The example is `after_create`.

1. Add a context struct to `scylla-extension/src/context.rs` if no existing context is correct. Give it the name of the hook, for example `AfterCreateContext { kind, owner_id, id }`. Copy the shape of `DeleteContext`. Export it from `scylla-extension/src/lib.rs`.

2. If the hook needs the id of the resource, add `pub struct ResourceId(pub Uuid);` with the derives of `UserId` to `scylla-domain/src/resource.rs`, and export it from `scylla-domain/src/lib.rs`. The core service gives `ResourceId(<entity>.id.0)`.

3. Add the method to `scylla-extension/src/guard.rs`, with a default body:

   ```rust
   async fn after_create(
       &self,
       _tx: &mut Transaction<'_, Postgres>,
       _ctx: &AfterCreateContext,
   ) -> Result<(), GuardError> {
       Ok(())
   }
   ```

   Use the name `before_<op>` if the hook can refuse the operation. Use `after_<op>` if the hook only follows it.

4. If `GuardChain` exists (see "Several guards"), add the method to `GuardChain`. Without it, the chain uses the no-op default and does not call its guards.

5. In each core service of the operation, call the hook in the transaction. Call `before_<op>` before the store write. Call `after_<op>` after the store write and before the commit.

6. In the core unit tests of each feature that calls the hook, add a test with a guard that records the context. Copy the shape of `RecordDeletes` and `delete_gives_the_kind_and_the_owner_to_the_guard` in `scylla-core/src/features/projects/tests.rs`.

7. The Enterprise repository implements the hook, if an EE feature needs it, after it moves to the new tag. Tell the user in the report.

8. In `README.md`, section "Extension points", add the hook and the time when it runs.

## A new value in a context

1. Add the `pub` field to the context struct, and a parameter to its `new()`.
2. The compiler shows each call of `new()`: the core services and the core tests. Change each one.
3. This change breaks the Enterprise repository when it moves to the new tag. Tell the user in the report.

## A new trait

Do not make a new trait for an action before or after a create or a delete. Use a hook on `ResourceGuard`. A new trait is for an extension that is not about a resource write. The example is an `Authenticator` that finds the caller of each request.

1. Create `scylla-extension/src/<name>.rs` with the trait and its no-op implementation (for example `NoAuthentication`). Add `mod <name>;` and the `pub use` line to `scylla-extension/src/lib.rs`.
2. Add a field `pub <name>: Arc<dyn <Trait>>` to `Edition` in `scylla-core/src/app.rs`.
3. In `routes()`, give the field to the features that call it. They give it to the service functions as `&dyn <Trait>`.
4. Set the field in `scylla-ce/src/app.rs` with the no-op implementation. The Enterprise repository must set it in its `app.rs` when it moves to the new tag. Tell the user in the report.
5. Add a core unit test with an implementation that records the calls.
6. In `README.md`, section "Extension points", add the trait.

## Several guards

`Edition` has one `guard`. The second guard feature adds a composite to `scylla-extension/src/guard.rs`:

```rust
pub struct GuardChain(pub Vec<Arc<dyn ResourceGuard>>);

#[async_trait]
impl ResourceGuard for GuardChain {
    async fn before_create(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        ctx: &CreateContext,
    ) -> Result<(), GuardError> {
        for guard in &self.0 {
            guard.before_create(tx, ctx).await?;
        }
        Ok(())
    }

    async fn after_delete(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        ctx: &DeleteContext,
    ) -> Result<(), GuardError> {
        for guard in &self.0 {
            guard.after_delete(tx, ctx).await?;
        }
        Ok(())
    }
}
```

- The first refusal stops the chain. The core then rolls back the transaction, so the writes of the guards that ran before it are also removed.
- `GuardChain` must implement each method of `ResourceGuard`, also each hook that you add later. A method that it does not implement uses the no-op default, and the guards in the chain are not called.
- Export `GuardChain` from `lib.rs`.
- An edition with two guards then uses `guard: Arc::new(GuardChain(vec![<first guard>, <second guard>]))` in its `app.rs`.
- Add core unit tests with two recording guards. One test calls each method of `ResourceGuard` through the chain, and makes sure that each guard gets the call. One test makes the first guard refuse, and makes sure that the second guard is not called.

## How an edition adds a gRPC service

This needs no contract change. `routes()` in `scylla-core/src/app.rs` adds the core services to a `RoutesBuilder`, then calls `edition.services.add_services(&mut routes, &pool)`. The Community edition gives `NoExtraServices`. The Enterprise edition gives its own implementation. To add an EE service, work in the Enterprise repository.

Two editions cannot serve the same service name. If an edition adds a service with the name of a core service, the router panics when the server starts. The integration tests start the server, so they find this.

## Finish

Load the `finish-change` skill and do all of it.
