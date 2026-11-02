---
name: add-rpc
description: Add an operation to an existing Scylla core feature, for example GetProject, RenameProject or ArchiveProject. Use when the user asks for a new gRPC call on a core resource that already exists. For a new resource, use add-core-feature. For a new field on an entity, use add-field. For an RPC on an Enterprise service (for example QuotaService), work in the Enterprise repository.
---

# Add an RPC to an existing core feature

The example is `RenameProject` on the feature `projects`. Replace these names:

- `<feature>`, `<entity>`, `<Entity>`, `<Feature>`: as in the `add-core-feature` skill (`projects`, `project`, `Project`, `Projects`).
- `<Op>`: the RPC name (`RenameProject`).
- `<op>`: the verb in snake case, without the entity (`rename`). The store function, the service function and the `impl` method have this name.

## Decisions that are already made

- Make one RPC for each intent. Make `RenameProject` with `project_id` and `name`, not a general `UpdateProject`. Use `UpdateProject` with a `google.protobuf.FieldMask` only if the user asks for it.
- Each RPC has its own `<Op>Request` and `<Op>Response`. A write returns the changed entity in the response (`Project project = 1;`). A delete returns an empty response.
- A read also uses a transaction. The service begins it and commits it.
- If no row has the id, the result is `Error::NotFound("<entity>")`. The client gets `NOT_FOUND`.
- Create calls `guard.before_create`. Delete calls `guard.after_delete`. Other writes do not call the guard. If an edition must act on the new operation, the guard has no hook for it. Load the `change-extension-point` skill first and add the hook.
- The POC has no authentication. Do not add an access check. If the request names a role (for example "an admin"), write `No access check.` at the end of the RPC comment, and tell the user in the report.

## Steps

1. **Proto.** Load the `edit-proto` skill and do "Add an RPC". Add the RPC to `proto/scylla/<feature>/v1/<entity>_service.proto`.

2. **Domain input.** If the operation takes more than one value, add an input struct to `scylla-domain/src/<feature>.rs`. Example: `RenameProject { id: ProjectId, name: ProjectName }`. Use the value types for validated input. If one id is sufficient, do not add a struct.

3. **Store.** Add the function `<op>` to `scylla-core/src/features/<feature>/store.rs`. It takes `tx: &mut Tx<'_>` first and returns `Result<_, Error>`. To change one row, use `UPDATE ... WHERE id = $1 RETURNING <columns>` with `fetch_optional`, and return `Option<<Entity>>`.

4. **Service.** Add `pub async fn <op>(pool: &PgPool, ...)` to `service.rs`. Begin, call the store, change `None` into `Error::NotFound("<entity>")`, then commit. Add the parameter `guard: &dyn ResourceGuard` only if the operation calls the guard.

5. **gRPC.** In `grpc.rs`:
   - Add the trait method with one line: `Ok(Response::new(self.<op>(request.into_inner()).await?))`. The code does not compile until this method exists, because tonic makes each RPC a required method of the service trait.
   - Add the method `async fn <op>(&self, request: pb::<Op>Request) -> Result<pb::<Op>Response, Error>` to `impl <Feature>Grpc`.
   - Parse each id with `convert::uuid(&request.<x>_id, "<x>_id")?`. Parse each value with `<Value>::parse(...)?`.
   - Make the response with `to_pb`.

6. **SQL cache.** Run `just prepare`.

7. **Test kit.** Add the helper `<op>_<entity>` (the RPC name in snake case) to `scylla-testkit/src/app.rs`. It sends the request and returns `Result<proto type, Status>`. Copy the shape of `create_project`.

8. **Tests.** Load the `write-tests` skill.
   - Add an integration test to `scylla-ce/tests/api.rs` for the success case.
   - Add each error case of the operation to `rejects_invalid_<entity>_requests`: `INVALID_ARGUMENT` for an invalid id or value, and `NOT_FOUND` for an unknown id.
   - Add a core unit test to `features/<feature>/tests.rs` only if the service has logic that the integration test cannot see. Examples: a guard call, a rollback, a page boundary.

9. **Docs.** Do not add the RPC to the section "Run" of `README.md`. The proto file is the API reference.

10. **Finish.** Load the `finish-change` skill and do all of it.
