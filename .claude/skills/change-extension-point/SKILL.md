---
name: change-extension-point
description: Change the contract between the Scylla core and its editions in scylla-extension, which means a new hook event, a new value in a hook context, a new builder method on EditionBuilder, a new ServiceContext value, or a change to authentication. Use when an edition must act on something that the core does not show yet. Never put edition code in the core instead.
---

# Change an extension point

The contract is `scylla-extension`. Each change there is a change to the API for the editions: tell the user, because the Enterprise build can stop at the next tag.

## Rules

- General words only: resource, kind, actor, owner, hook, plugin, capability. Never an edition word.
- The core holds implementations as `Arc<dyn Trait>`. The core is never generic over an edition.
- A hook gets `&mut Tx`: it never commits. It writes only to its own tables.
- A context struct is `#[non_exhaustive]` with a `new()`.
- A new extension point is a new `EditionBuilder` method. Never add a public field to `Edition`.

## A new hook event

1. A context in `context.rs` (copy `CreateContext`), and a trait with one method in `hooks.rs` (copy `AfterDelete`).
2. A map in `Hooks`, its run method, and its `add_` method. A builder method in `edition.rs`.
3. Call the run method in each core service of the operation: `before_` before the store write, `after_` after it, both before the commit.
4. Core unit tests with a recording hook: it gets the call, the order holds, a refusal rolls back.

## A new context value

Add the field and a parameter to `new()`. Change each call in the core. Tell the user.

## Access

The `Gate` trait is the access contract: `authenticate`, `authorize`, `issue`, `write`, `delete`, `delete_object`, and the helper `caller`. A new way to sign in is a new exchange in `TokenService` that ends with `gate.issue`. A new kind of object is an `authz!` definition, registered with `EditionBuilder::authz`. Ownership stays in Postgres: `Rights` reads the owner chain, and SpiceDB holds only grants. Never add a SpiceDB write to a creation. An edition adds to a core definition with `authz! { extend ... }`. A new form of the schema language goes in `scylla-macros` (parse, checks, tests) and in `scylla_domain::authz::Schema`. Each route checks its own permission (the RPC name in snake case): `Access::authorize` and `OperationLayer` keep this rule. Never add a way to skip the token, the permission check or the route rule, or to make a `Check` or a `Relationship` from text.

## Finish

Load the `finish-change` skill. Add or change the entry in `DECISIONS.md`.
