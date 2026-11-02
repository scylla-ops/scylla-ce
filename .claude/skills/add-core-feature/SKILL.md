---
name: add-core-feature
description: Add a new feature to the Scylla core, which is a new resource with its own table, domain types and gRPC service (for example "pipelines"). Use when the user asks for a new entity, resource or service in the public edition. For an operation on an existing core feature, use add-rpc. For a new field, use add-field. For an Enterprise feature or an Enterprise RPC, work in the Enterprise repository.
---

# Add a core feature

## Names

This skill uses the feature `pipelines` as the example. Replace the names:

| Placeholder | Form | Example |
|---|---|---|
| `<feature>` | plural, snake case | `pipelines` |
| `<entity>` | singular, snake case | `pipeline` |
| `<Entity>` | singular, Pascal case | `Pipeline` |
| `<Feature>` | plural, Pascal case | `Pipelines` |

The feature name is also the table name, the proto package segment, the domain file name and the feature folder name.

The reference feature is `projects`. When a step says "copy the shape", open the `projects` file with the same name and copy its structure. Do not invent a new structure.

## Decisions that are already made

Do not discuss these again. If the user asks for something different, tell the user which rule changes, and update this skill.

- If the user names no fields, the entity has `id`, `owner_id`, `name` and `created_at`. The name has the rules of `ProjectName`: trimmed, 1 to 100 characters. If the user gives other limits, use them.
- Add no unique constraint, unless the user asks for one.
- The POC has no authentication. Do not add an access check. If the request names a role (for example "an owner can delete"), write `No access check.` at the end of the comment of each RPC, and tell the user in the report.
- A feature has two places. The pure types go in `scylla-domain/src/<feature>.rs`. The code with IO goes in `scylla-core/src/features/<feature>/`.
- The id is `<Entity>Id(pub Uuid)`. The store makes it with `Uuid::now_v7()`, so the ids sort in creation order.
- A resource that an owner creates has `owner_id: UserId`, a foreign key to `users (id)`, and a `KIND`. Its service calls `guard.before_create` on create and `guard.after_delete` on delete. Then an edition can limit it without a core change.
- A validated input value is a value type with `parse` in the domain crate. Its errors are variants of `DomainError`.
- The entity that the store returns uses the plain type (`String`), because the database holds only valid values.
- Each service function opens a transaction, also for a read. A store function takes `&mut Tx<'_>`. It never opens or commits a transaction.
- Store and service are free functions in modules. Do not make a struct for them. The transaction is a parameter, so each line shows which transaction it uses.
- Import the module, not the function: `use super::store;`, then call `store::insert(...)`.
- A list uses keyset pagination on the id. A `page_size` of 0 means 50. A value above 100 becomes 100. The `page_token` is the last id of the previous page.
- A conversion to a proto type is a free function `to_pb` in `grpc.rs`.
- Each RPC of a core service has one helper in `scylla-testkit`. The helper has the name of the RPC in snake case.

## Steps

Do the steps in this order. From step 4, run `cargo build --workspace` after each step. Until step 9, the build shows `dead_code` warnings. Step 9 removes them.

1. **Proto.** Load the `edit-proto` skill and do "Add a service". The result is `proto/scylla/<feature>/v1/<entity>.proto`, `<entity>_service.proto`, and the two registrations in `scylla-proto`.

2. **Domain types.** Create `scylla-domain/src/<feature>.rs`. Copy the shape of `scylla-domain/src/projects.rs`. Put the items in this order:
   - `pub const KIND: ResourceKind = ResourceKind::new("<entity>");`
   - `<Entity>Id`, with the same derives as `ProjectId`.
   - Each validated value type, with its limits as associated constants (like `MAX_LEN`), `parse` and `as_str`.
   - `<Entity>`: the entity as the store returns it.
   - `New<Entity>`: the input of create.
   - `List<Feature>` and `<Entity>Page`: the input and the output of list.

   Add `pub mod <feature>;` to `scylla-domain/src/lib.rs`, in alphabetical order. Add the error variants at the end of `DomainError` in `scylla-domain/src/error.rs`. Write each error message in lower case.

3. **Migration.** Load the `edit-migrations` skill. Do "Make the file" and "Add a table". Then run `just migrate`. The `query!` macros of step 4 need the new table in the database.

4. **Store and module.** Create `scylla-core/src/features/<feature>/mod.rs` with only the line `mod store;`. Add `pub(crate) mod <feature>;` to `scylla-core/src/features/mod.rs`, in alphabetical order. Create `store.rs`. Copy the shape of `projects/store.rs`, with `insert`, `list_by_owner` and `delete`.
   - `insert` changes a foreign key violation into `Error::NotFound("owner")`.
   - `delete` returns `Option<UserId>`. This is the owner of the deleted row, or `None` if no row was deleted. The service gives the owner to the guard.

5. **Service.** Create `service.rs`, and add `mod service;` to `mod.rs`. Copy the shape of `projects/service.rs`, with `create`, `list` and `delete`.
   - `create`: begin, then `guard.before_create(&mut tx, &CreateContext::new(KIND, new.owner_id))`, then `store::insert`, then commit.
   - `delete`: begin, then `store::delete`. `None` becomes `Error::NotFound("<entity>")`. Then `guard.after_delete(...)`, then commit.
   - `list`: get `limit + 1` rows. If there are more than `limit` rows, remove the last row and set `next_after` to the last id that stays.

6. **gRPC.** Create `grpc.rs`. Add `mod grpc;` and `pub(crate) use grpc::server;` to `mod.rs`. Copy the shape of `projects/grpc.rs`.
   - `pub fn server(pool: PgPool, guard: Arc<dyn ResourceGuard>) -> <Entity>ServiceServer<<Feature>Grpc>`.
   - `pub struct <Feature>Grpc { pool, guard }`.
   - Each trait method has one line: `Ok(Response::new(self.<op>(request.into_inner()).await?))`.
   - Each method of `impl <Feature>Grpc` has the name of the service function that it calls. It returns `Result<pb::<Op>Response, Error>`. It parses the ids with `convert::uuid(&request.<x>_id, "<x>_id")?` and the values with `<Value>::parse(request.<x>)?`. Then it calls the service and makes the response with `to_pb`.
   - Page size: if `page_size` is still in `projects/grpc.rs`, move the function `page_size` and the constants `DEFAULT_PAGE_SIZE` and `MAX_PAGE_SIZE` to the end of `scylla-core/src/convert.rs`. Make the function `pub`. Keep the constants private. Use `convert::page_size` in the two features. If `page_size` is already in `convert.rs`, use it.

7. **Unit tests.** Create `tests.rs`, and add `#[cfg(test)]` and `mod tests;` to `mod.rs`. Load the `write-tests` skill and do "Core unit tests".

8. **Module file.** Make sure that `mod.rs` has exactly this text:

   ```rust
   mod grpc;
   mod service;
   mod store;

   #[cfg(test)]
   mod tests;

   pub(crate) use grpc::server;
   ```

9. **Wiring.** In `scylla-core/src/app.rs`, add `<feature>` to the line `use crate::features::{...};`, in alphabetical order. Then add the service to `routes()`, before the call to `edition.services`:

   ```rust
   routes
       .add_service(projects::server(pool.clone(), edition.guard.clone()))
       .add_service(<feature>::server(pool.clone(), edition.guard.clone()))
       .add_service(capabilities::server(edition.capabilities.clone()));
   edition.services.add_services(&mut routes, &pool);
   ```

10. **Test kit and integration tests.** Load the `write-tests` skill and do "Integration tests". Add the client of the new service to `TestApp`, and one helper for each RPC.

11. **Docs.** In `README.md`:
    - In the tree, add the domain file, the feature folder, the proto files and the migration. Change the line of each file that you changed, like `convert.rs`.
    - In the table of the section "Tests", add the new tests.
    - Do not add the RPCs to the section "Run". The proto files are the API reference.
    - Do not change the recipe "Add a feature" unless a rule changed.

12. **Finish.** Load the `finish-change` skill and do all of it.

## When the Enterprise edition must act on the new resource

The guards of an edition return `Ok` for each kind that they do not handle. So the new resource has no limit in EE. If the user wants a limit, this is a separate task in the Enterprise repository, after a new tag of this repository. Tell the user.
