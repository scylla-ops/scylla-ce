---
name: add-field
description: Add a field to an existing Scylla entity from end to end, through the migration, the domain type, the proto message, the store, the gRPC conversion and the tests. Use when the user asks to store and show a new attribute of a resource, for example a description on projects.
---

# Add a field to an entity

The example is `description` on `projects`. Replace `<feature>`, `<entity>`, `<Entity>` and `<field>`.

## Decisions that come from the request

Find the answers in the request of the user. If the request does not give an answer, use the default.

| Question | Default |
|---|---|
| Can the client set the field on create? | Yes. |
| Is the field required? | No. An empty value is valid. |
| Does the server validate the field? | Yes, if it is text: a value type with a maximum length. |
| What is the maximum length of a text field? | 1000 characters, after trim. |
| Is the field in the list response? | Yes. The list returns the same entity. |

## Steps

Do the steps in this order. After steps 2 and 3, the compiler shows the places to change.

1. **Migration.** Load the `edit-migrations` skill. Do "Make the file" and "Add a column". Then run `just migrate`. The existing rows need a value, so the column has a `DEFAULT` or accepts `NULL`. For optional text, use `text NOT NULL DEFAULT ''`.

2. **Domain.** In `scylla-domain/src/<feature>.rs`:
   - Add the field to `<Entity>` with the plain type: `String`, `i64`, `OffsetDateTime` or `Option<T>`.
   - If the client sets the field, add it to `New<Entity>` with its value type.
   - If the server validates the field, add a value type with `parse`, with the shape of `ProjectName`. If the field is not required, the value type accepts an empty string: do not copy the empty check of `ProjectName`. Add its error variants at the end of `DomainError` in `scylla-domain/src/error.rs`.

3. **Proto.** Load the `edit-proto` skill and do "Add a field". Add the field to the entity message. If the client sets it, also add it to the request message. Use the next free field number. Never change the number of an existing field.

4. **Store.** In `store.rs`, add the column to each `INSERT`, `SELECT` and `RETURNING` of the table, and to each mapping to `<Entity>`. SQL in other crates can also read the table: the test kit and the tests. Find all of it with this command:

   ```sh
   grep -rn '<feature>' --include='*.rs' scylla-*/src scylla-*/tests
   ```

   The command also shows the `use` lines and the SQL that reads only other columns. Change only the SQL that must return the new column, and the code that maps its rows to `<Entity>`.

   The Enterprise repository can also read the table. A new column with a `DEFAULT` does not break its SQL. A required field that changes `New<Entity>` or a helper of the test kit breaks its build at the next tag. Tell the user in the report.

5. **gRPC.** In `grpc.rs`, parse the field from the request with `<Value>::parse(request.<field>)?`. Set the field in `to_pb`.

6. **Test kit.** The compiler shows each struct literal of the request type that has no value for the new field. Add the field to each one.
   - If the field is not required, the existing helpers send the default value. Add a new helper `create_<entity>_with` that also takes the field. Do not change the parameters of an existing helper.
   - If the field is required, add a parameter to `create_<entity>`, and change each call.

7. **SQL cache.** Run `just prepare`.

8. **Tests.** Load the `write-tests` skill.
   - Add the integration test `creates_a_<entity>_with_a_<field>` to `scylla-ce/tests/api.rs`. It creates an entity with the field, and then reads it back with list.
   - If the server validates the field, add its limits to `rejects_invalid_<entity>_requests`, with `Code::InvalidArgument`.

9. **Docs.** Write the comment of the field in the proto file. Change `README.md` only if the field changes a rule.

10. **Finish.** Load the `finish-change` skill and do all of it.
