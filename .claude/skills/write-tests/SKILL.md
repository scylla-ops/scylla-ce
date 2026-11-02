---
name: write-tests
description: Where and how to write the Scylla core tests, which means core unit tests, gRPC integration tests with scylla-testkit, concurrency tests, and the check that a protection test can fail. Use when you add or change a test, a test helper or TestApp. Other skills load it.
---

# Write tests

All tests use a real Postgres. `sqlx::test` makes a new database for each test. Run `just migrate` first: it starts the database and applies the migrations that the `query!` macros need to compile.

## Where a test goes

| What the test checks | File | Attribute |
|---|---|---|
| The service and store logic of a core feature, with a test guard | `scylla-core/src/features/<feature>/tests.rs` | `#[sqlx::test(migrator = "crate::db::MIGRATOR")]` |
| The gRPC behavior of the Community edition | `scylla-ce/tests/api.rs` | `#[sqlx::test(migrations = false)]` |

- An integration test uses `migrations = false`, because `TestApp::spawn` runs the core and edition migrations.
- Do not add a second file to a `tests/` folder. Each file is one more test binary to compile and link. Put new integration tests in `tests/api.rs`.
- Test the validation of a domain type through gRPC, in `rejects_invalid_<entity>_requests`.

## Style

- The test name tells the expected behavior, in snake case: `delete_frees_a_slot`, `a_rejection_rolls_back_the_guard_writes`.
- A test has three parts, with an empty line between them: prepare, act, check.
- Setup errors use `unwrap()` or `expect("...")`. The value under test uses `unwrap_err()`, `assert!` or `assert_eq!`.
- When the feature has strategies, write one `async fn` with the strategy as a parameter. Then write one `#[sqlx::test]` for each strategy that calls it.
- Do not write `//` comments. The name and the asserts tell what the test checks.

## Helpers

- Each RPC of a core service has one helper in `scylla-testkit/src/app.rs`. The helper has the name of the RPC in snake case (`create_project`, `get_capabilities`). It returns `Result<proto type, Status>`, and uses `expect` only for its setup. Copy `create_project`.
- Tests call the helpers. A test that must send a request that the helper cannot make (for example an id that is not a UUID) uses the client directly (`app.projects.clone()`).
- For a new core service, add its client as a `pub` field of `TestApp`. Make it from the same `channel` in `spawn`.
- `scylla-testkit` is part of the API for the editions. The Enterprise tests use `TestApp`, its helpers and its `channel`. Do not change the parameters of an existing helper. Add a new helper.

## Core unit tests

Copy the shape of `scylla-core/src/features/projects/tests.rs`:

- `owner(&pool)` makes a user with `users::insert`, in a committed transaction.
- Test guards are small structs in the test file: `AllowAll` (from `scylla_extension`), a guard that writes and then refuses (`WriteThenReject`), and a guard that records its contexts.
- Call `service::<op>` directly.
- Count rows with `sqlx::query_scalar!(r#"SELECT count(*) AS "count!" FROM <table>"#)`.

For each new feature, write at least these three tests:

- `a_rejection_rolls_back_the_guard_writes`: copy it, with the service of the new feature.
- The guard gets the correct `KIND` and owner on create and on delete. Make one recording guard (`RecordContexts`) with one list for the create contexts and one list for the delete contexts.
- List returns all the rows in creation order, on more than one page.

## Integration tests

- Make the app with the local `app(pool)` function at the top of `tests/api.rs`: `TestApp::spawn(pool, scylla_ce::edition())`.
- For a new feature, write `creates_lists_and_deletes_<feature>`. It also makes rows for a second owner, and checks that the list of one owner does not contain them.
- For a new feature, write `rejects_invalid_<entity>_requests`. It checks each limit at the boundary: `MAX_LEN` passes, and `MAX_LEN + 1` fails. It also checks an id that is not a UUID (`INVALID_ARGUMENT`), an unknown owner and an unknown id (`NOT_FOUND`), and a negative page size (`INVALID_ARGUMENT`).
- Check the error code with `status.code()`.
- Check a refusal of an edition with `status.get_details_error_info()` (from `tonic_types::StatusExt`): `reason`, `domain == "scylla"` and the `metadata` values.

## Concurrency tests

Write a concurrency test when a rule must stay true while transactions run at the same time: a limit, a counter, a unique value.

- The test takes `(_options: PgPoolOptions, connect: PgConnectOptions)`. It makes its own pool with `max_connections(ATTEMPTS as u32 + 5)`. The default pool of `sqlx::test` has 5 connections, so the transactions would not run at the same time.
- Use `ATTEMPTS = 20` and a limit of 10, so the expected result is exactly 10 accepted and 10 refused.
- Put the app in an `Arc`. The tasks wait on `tokio::sync::Barrier::new(ATTEMPTS)`, then send the requests together.
- Assert the exact numbers. Then count the rows in the database.
- At the end, call `pool.close().await`.

## Make sure that a test can fail

A test of a protection must fail when the protection is not there. Do this one time for each new protection:

1. Remove the protection, for example the `FOR UPDATE`, or the condition of the `UPDATE`.
2. Run the test 3 times with `cargo test -p scylla-ce --test api <test name>`. It must fail.
3. Put the protection back, and run the test again. It must pass.
4. Write the result in the report. If the protection is important, also write it in `README.md`, section "Tests".

## Run

- All tests: `just test`.
- One test: `cargo test --workspace <part of the name>`. For an integration test: `cargo test -p scylla-ce --test api <name>`.
- Use `-p` only with `scylla-ce`. With another crate, Cargo builds a second copy of tokio and sqlx.
