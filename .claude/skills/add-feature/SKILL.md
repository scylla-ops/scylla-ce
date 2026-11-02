---
name: add-feature
description: Add a core feature (a new resource with its table, domain types, plugin and gRPC service), an RPC on an existing core feature, or a field on an entity, in the Scylla public repository. Use for each change to the core API or schema. For the contract between core and editions, use change-extension-point. For an Enterprise feature, work in the Enterprise repository.
---

# Add a feature, an RPC or a field

Read `AGENT.md` first. The reference feature is `projects`: copy the shape of its files. The example is a feature `pipelines` (`<feature>`, `<entity>` = `pipeline`, `<Entity>` = `Pipeline`).

## New feature

1. **Proto.** `proto/scylla/<feature>/v1/<entity>.proto` and `<entity>_service.proto`. Ids are `scylla.common.v1.Id`, lists use `PageRequest` and `PageResponse`. A comment on each item. Add the files to `scylla-proto/build.rs` and a module to `scylla-proto/src/lib.rs`. Run `buf lint`.
2. **Domain.** `scylla-domain/src/<feature>.rs`: `KIND`, `pub type <Entity>Id = Id<<Entity>>;`, value types with `parse`, the entity, `New<Entity>`, `List<Feature>`, the page type. New error variants go in `DomainError`, with the reason only.
3. **Migration.** `sqlx migrate add --sequential --simple --source scylla-core/migrations <feature>`. Columns `id uuid PRIMARY KEY`, `owner_id` and `created_by` `uuid NOT NULL REFERENCES actors (id)` with an index each, `created_at timestamptz NOT NULL DEFAULT now()`. Run `just migrate`.
4. **Store.** `store.rs`: `insert`, `list_by_owner`, `delete`, on `&mut Tx<'_>`, with `to_db()` and `from_db()` for ids. Map the owner foreign key to `Error::NotFound("owner")`.
5. **Service.** `service.rs`: begin, `hooks.before_create` or `hooks.after_delete` with the context, store, commit.
6. **gRPC.** `grpc.rs`: `gate.authenticate`, then `gate.authorize` with a `Check` (operation `<feature>.<verb>`, `act_for` on the owner, or a permission on the object), parse with `parse::required`, `parse::or` (owner defaults to the caller), `parse::field`, `parse::optional`. Answer with `ToProto`.
7. **Permissions.** Add the definition of the new object to `CORE_SCHEMA` (`scylla-core/src/access/schema.rs`), with `owner: actor` and permissions through `owner->act_for`. The service writes the owner relation with `gate.write` before the commit, commits with `access::commit`, and calls `gate.delete_object` after a delete.
8. **Plugin.** `plugin.rs`: register the service with `ctx.pool()`, `ctx.hooks()`, `ctx.gate()`. Install it in `scylla-core/src/app.rs`.
9. **Tests.** One helper per RPC in `scylla-testkit` (with `app.authed`). A module `scylla-ce/tests/api/<feature>.rs`: success, each error code, the boundaries of each value, the owner default, and a refusal for an actor with no permission. A unit test only for what the API cannot show.
10. **Finish.** `just prepare`, then the `finish-change` skill.

## New RPC on a feature

Proto (request and response messages), store function, service function (one transaction), handler method, testkit helper, API tests. If an edition must act on it, first add the hook with `change-extension-point`.

## New field on an entity

New migration (`ALTER TABLE ... ADD COLUMN` with a default), domain field (and value type if the server checks it), proto field with the next free number, store SQL, `ToProto`, request parsing, API test. A required field in `New<Entity>` breaks the Enterprise build at the next tag: tell the user.
