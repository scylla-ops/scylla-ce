---
name: edit-proto
description: Rules and steps to change the public Scylla gRPC contract in proto/, with the codegen crate scylla-proto. This means a new service, a new RPC, a new field, or the removal of a field. Use for each change to a .proto file in this repository. Other skills load it.
---

# Edit the protos

Load the user skill `writing-protos` only if the message needs states (`oneof`), an enum or an `optional` field, and only if the skill is available. When the two skills do not agree, use the rules below. For a message with the shape of the `projects` messages, do not load it.

## Layout

- `proto/` is the only include root of this repository, and `scylla-proto` is its codegen crate. `buf.yaml` lists it as a module.
- The Enterprise repository has its own include root and its own codegen crate. Its protos never import a core message. So a change to a core proto cannot break an EE proto.
- The package is `scylla.<feature>.v1`, in the folder `proto/scylla/<feature>/v1/`.
- An EE package has the same form, with no `ee` in its name. Then a feature can move from EE to the core with no change to its API. Before you add a package, make sure that the Enterprise repository does not use its name. Ask the user if you do not know.
- `<entity>.proto` holds the entity message. `<entity>_service.proto` holds the service and its request and response messages.
- The generated code goes to `OUT_DIR`. It is not in the repository.
- `protoc` must be installed.

## Message rules

- Each RPC has its own `<Op>Request` and `<Op>Response`, also when the message is empty (`message DeleteProjectResponse {}`). Never use `google.protobuf.Empty`: `buf lint` refuses it, and a later field would change the response type, which breaks each client.
- A response that gives an entity wraps it: `Project project = 1;`.
- An id is a `string` with a UUID in text form. Its name is `id` in its own entity, and `<entity>_id` in other messages.
- A time is a `google.protobuf.Timestamp`. The Rust code uses `convert::timestamp`.
- A list request has `int32 page_size` and `string page_token`. A list response has `repeated <Entity> <feature>` and `string next_page_token`. Copy `ListProjectsRequest` and `ListProjectsResponse` with their comments.
- Each service, RPC, entity message and field has a `//` comment of one line. prost changes these comments into Rust doc comments. Request and response messages do not need a comment on the message line.
- The comment of an RPC that an edition can refuse ends with: "The edition can refuse it with FAILED_PRECONDITION and a google.rpc.ErrorInfo detail."
- The comment of an RPC that the request gives to a role (for example "an admin") ends with: "No access check."
- Field numbers 1 to 15 use one byte. Give them to the fields that are always set.

## Errors, as the server sends them

| Case | Code | Rust source |
|---|---|---|
| Invalid id, value or page size | `INVALID_ARGUMENT` | `DomainError`, `Error::InvalidArgument` |
| Unknown id | `NOT_FOUND` | `Error::NotFound("<entity>")` |
| Refusal by the edition | `FAILED_PRECONDITION` with `ErrorInfo { reason: code, domain: "scylla", metadata: details }` | `Error::Rejected`, from a guard or an EE service |
| All other errors | `INTERNAL`, message "internal error". The cause goes to the log. | the other `Error` variants |

`Error` is `scylla_core::Error`, for the core routes and for the EE routes. For a new case, add a variant to `scylla-core/src/error.rs` and its line in `From<Error> for Status`.

## Add a service

1. Create `proto/scylla/<feature>/v1/<entity>.proto` and `<entity>_service.proto`. Copy the shape of the `projects` files: `syntax`, `package`, the imports in alphabetical order, then the definitions.
2. In `scylla-proto/build.rs`, add the two paths to the list, in alphabetical order.
3. In `scylla-proto/src/lib.rs`, add this module, in alphabetical order:

   ```rust
   pub mod <feature> {
       pub mod v1 {
           tonic::include_proto!("scylla.<feature>.v1");
       }
   }
   ```

4. Run `buf lint`, then `cargo build --workspace`.

These are the generated names for the Rust code:

- `use scylla_proto::<feature>::v1 as pb;`
- The server: `scylla_proto::<feature>::v1::<entity>_service_server::{<Entity>Service, <Entity>ServiceServer}`.
- The client: `scylla_proto::<feature>::v1::<entity>_service_client::<Entity>ServiceClient`.
- A trait method is the RPC name in snake case. `RenameProject` becomes `rename_project`.

## Add an RPC

1. Add the `rpc` line to the service, with its comment.
2. Add `<Op>Request` and `<Op>Response` after the other messages of the service file.
3. Run `buf lint`.

## Add a field

1. Add the field at the end of the message, with the next free number and a comment.
2. If "not set" must be different from the zero value (`0`, `""`, `false`), write `optional`. prost then makes an `Option<T>`.
3. Run `buf lint`.

## Remove or change a field

On the wire, the field number is the identity of the field. An old client reads a reused number as the old field.

- Never change the number or the type of a field. Never use a number again.
- To remove a field, delete it and add `reserved <number>;` and `reserved "<name>";` to the message.
- Do not move a message to another file. `buf breaking` in `FILE` mode refuses it.
- A change that breaks clients needs a new package `scylla.<feature>.v2`. Ask the user before you do this.
- When the repository uses Git, run `buf breaking --against '.git#branch=main'`.
