---
name: add-feature
description: Add a core feature (a new resource with its table, domain types, plugin and gRPC service), an RPC on an existing core feature, or a field on an entity, in the Scylla public repository. Use for each change to the core API or schema. For the contract between core and editions, use change-extension-point. For an Enterprise feature, work in the Enterprise repository.
---

# Add a feature, an RPC or a field

Read `AGENT.md` first. The reference feature is `projects`: copy the shape of its files. The example is a feature `pipelines` (`<feature>`, `<entity>` = `pipeline`, `<Entity>` = `Pipeline`).

## New feature

1. **Proto.** `proto/scylla/<feature>/v1/<entity>.proto` and `<entity>_service.proto`. Ids are `scylla.common.v1.Id`, lists use `PageRequest` and `PageResponse`. A comment on each item. Add the files to `scylla-proto/build.rs` and a module to `scylla-proto/src/lib.rs`. Run `buf lint`.
2. **Domain.** `scylla-domain/src/<feature>.rs`: `KIND` (`ResourceKind::new(<Entity>::NAME)`), `pub type <Entity>Id = Id<<Entity>>;`, value types with `parse`, the entity, `New<Entity>`, `List<Feature>`, the page type. New error variants go in `DomainError`, with the reason only.
3. **Migration.** `sqlx migrate add --sequential --simple --source scylla-core/migrations <feature>`. Columns `id uuid PRIMARY KEY`, `owner_id` and `created_by` `uuid NOT NULL REFERENCES actors (id)` with an index each, `created_at timestamptz NOT NULL DEFAULT now()`. Run `just migrate`.
4. **Store.** `store.rs`: `insert`, `owner`, `list_by_owner`, `delete`, on `&mut Tx<'_>`, with `to_db()` and `from_db()` for ids. Map the owner foreign key to `Error::NotFound("owner")`.
5. **Service.** `service.rs`: begin, `hooks.before_create` or `hooks.after_delete` with the context, store, commit. No SpiceDB write: the owner is in the row. After a delete, call `gate.delete_object(&Object::of(id))` to remove the grants.
6. **gRPC.** `grpc.rs`: `let caller = self.gate.caller(&request).await?;`, parse with `parse::required`, `parse::or` (owner defaults to the caller), `parse::field`, `parse::optional`, then `self.gate.authorize(&caller, <route permission>.on(id))`: `<Entity>Owner::CREATE_<ENTITY>.on(owner_id)` for a create or a list. On one object, read its owner with `service::owner` first (no owner gives the refusal), then `<Entity>::DELETE_<ENTITY>.on(id).owned_by(owner)`. Answer with `ToProto`.
7. **Permissions.** In the domain file, one `crate::authz!` block: `extend Actor as <Entity>Owner` with one permission for each route on the owner (`permission create_<entity> = act_for;`, `permission delete_<entity> = act_for;`). If one object can be shared, also `definition <entity> for <Entity>` with roles (`relation viewer: Actor;`), groups (`permission view = viewer + viewer->act_for;`) and the route permissions on the object. The name of a route permission is the RPC name in snake case.
8. **Plugin.** `plugin.rs`: `.authz::<<Entity>Owner>()` (and `.authz::<Entity>()` if the object has a definition), then the service with `ctx.pool()`, `ctx.hooks()`, `ctx.gate()`. Install it in `scylla-core/src/app.rs`.
9. **Tests.** One helper per RPC in `scylla-testkit` (with `app.authed`). A module `scylla-ce/tests/api/<feature>.rs`: success, each error code, the boundaries of each value, the owner default, and a refusal for an actor with no permission. Test the owner, a delegate and each role with `app.grant` and `app.can`. A unit test only for what the API cannot show.
10. **Finish.** `just prepare`, then the `finish-change` skill.

## New RPC on a feature

Proto (request and response messages), the route permission (the RPC name in snake case, in the `authz!` block of the feature, on a group), store function, service function (one transaction), handler method, testkit helper, API tests. If an edition must act on it, first add the hook with `change-extension-point`.

## New field on an entity

New migration (`ALTER TABLE ... ADD COLUMN` with a default), domain field (and value type if the server checks it), proto field with the next free number, store SQL, `ToProto`, request parsing, API test. A required field in `New<Entity>` breaks the Enterprise build at the next tag: tell the user.
