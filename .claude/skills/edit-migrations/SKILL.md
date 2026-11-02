---
name: edit-migrations
description: Rules and steps to change the Scylla core database schema and its SQL, which means a new table, a new column, a new index or constraint, and the sqlx query cache. Use for each new migration in scylla-core/migrations, and after each change to a sqlx query. Other skills load it.
---

# Edit migrations and SQL

## Facts about this repository

- The database runs in Docker on `127.0.0.1:55432`. `just migrate` starts it if necessary. `.env` gives `DATABASE_URL`.
- Port 5432 and the container `scylla-postgres` belong to another project. Do not touch them.
- The core migrations are in `scylla-core/migrations/`. Their ledger table is `_sqlx_migrations_core`.
- The Enterprise repository has its own migrations and its own ledger table, in the same database.
- The `sqlx.toml` of `scylla-core` sets its ledger table. Never change `table-name`.
- `db::migrate` runs the core migrations first, then the edition migrations.
- The `query!` macros check the SQL against the database when they compile. So the database must have each migration before the code that uses it compiles. `.sqlx/` keeps the result, for builds without a database (`SQLX_OFFLINE=true`). The offline build of the Enterprise edition also uses it.

## Rules

- Never change a migration that ran on a database. The ledger keeps a checksum of each file, and `migrate` fails when the file changes. Write a new migration.
- Do not write down migrations. To undo a change, write a new forward migration.
- A core migration never references an EE table. An EE migration can reference a core table (`REFERENCES users (id)`).
- The Enterprise SQL reads `users` and `projects`. A change that removes or renames a column of these tables breaks the Enterprise build at the next tag. Tell the user in the report.
- The table name is the feature name: plural, snake case.
- An entity table has `id uuid PRIMARY KEY`. The application makes the id with `Uuid::now_v7()`. Do not use a database default for an id.
- In an entity table, the owner column is `owner_id uuid NOT NULL REFERENCES users (id)`, with an index `<table>_owner_id_idx`.
- A table with one row for each owner and kind has no `id`. Its primary key is `(owner_id, kind)`. This key is also the index of the owner.
- A creation time is `created_at timestamptz NOT NULL DEFAULT now()`.
- Text uses the type `text`. The domain type checks the length, not the database.
- An invariant that concurrent writes must not break gets a `CHECK` constraint as the last barrier. Example: `CHECK (used >= 0 AND used <= quota_limit)` on a counter.
- Add an index for each column that a frequent `WHERE` or a foreign key uses.

## Make the file

Always use these flags. They give the next number of the folder:

```sh
sqlx migrate add --sequential --simple --source scylla-core/migrations <name>
```

`<name>` is in snake case. For a new table, it is the table name (`pipelines`). For a change, it tells the change (`add_description_to_projects`).

The command writes `-- Add migration script here` in the file. Replace this line with the SQL.

If two branches add the same number, give the newer file the next free number before the merge. Do this only if the file did not run on a database.

## Add a table

Copy the shape of `scylla-core/migrations/20261028000002_projects.sql`:

```sql
CREATE TABLE <feature> (
    id uuid PRIMARY KEY,
    owner_id uuid NOT NULL REFERENCES users (id),
    <columns>,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX <feature>_owner_id_idx ON <feature> (owner_id);
```

Then do "Apply and refresh".

## Add a column

The table has rows, so the new column needs a value for them:

```sql
ALTER TABLE <feature> ADD COLUMN <field> text NOT NULL DEFAULT '';
```

Accept `NULL` (no `NOT NULL`) only if "no value" has a meaning that is different from the default value. Then the Rust type is `Option<T>`.

Then do "Apply and refresh".

## Apply and refresh

1. Run `just migrate`. It starts the database if necessary, creates it, and runs the core migrations.
2. Write or change the `query!` calls.
3. Run `just prepare`. It refreshes `.sqlx/` for all targets, tests included.
4. Keep the `.sqlx/` changes in the same change as the code.

## SQL in Rust

- A store function takes `tx: &mut Tx<'_>`. The query runs with `.fetch_one(&mut **tx)`, `.fetch_optional(&mut **tx)`, `.fetch_all(&mut **tx)` or `.execute(&mut **tx)`.
- Give an id as `id.0`. Give a value type as `name.as_str()`.
- For sqlx, `count(*)` can be null. To get an `i64`, write `SELECT count(*) AS "count!"` in a raw string `r#"..."#`.
- For an optional filter, write `($2::uuid IS NULL OR id > $2)`.
- A foreign key violation on insert becomes `Error::NotFound("<parent>")`. Copy the `map_err` of `projects/store.rs`.
- If a client can cause another database error (a unique violation, a check violation), change it into an error that the client understands, in the same way. If no variant is correct, add one to `scylla-core/src/error.rs`.
- A rule that counts rows, or reads and then writes, must stay correct when transactions run at the same time in READ COMMITTED. Two patterns are correct: a lock on the parent row (`SELECT ... FOR UPDATE`) before the count, or a conditional `UPDATE ... WHERE <condition>` with a `CHECK` constraint. Write the reason in a `///` doc comment, and write a concurrency test (load the `write-tests` skill).
