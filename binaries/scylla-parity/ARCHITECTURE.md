# scylla-parity

`scylla-parity` sends one fixed scenario of gRPC calls to a server. It writes one
JSON line for each step. Two outputs must be equal when two servers give the same
answers to clients. The gRPC code is part of each line, because the clients act on
the codes (the web UI signs the user out on `UNAUTHENTICATED`, the agent stops on
`UNAUTHENTICATED`, `PERMISSION_DENIED`, `NOT_FOUND` and `ALREADY_EXISTS`).

## Commands

| Command | What it does |
|---|---|
| `just parity` | Builds `scylla-ce`, then runs `baseline`. Docker must run. |
| `just parity-acceptance <b> [args]` | Builds `scylla-ce`, then runs `baseline` with `--b-server <b>`. |
| `scylla-parity baseline --server <binary>` | For each run (default 2): starts a fresh Postgres container, starts the server on `127.0.0.1:<port>` (default 18080), runs the scenario and the contract, stops both. Then it compares the runs. |
| `scylla-parity run --addr <host:port> --database-url <url> --out <file>` | Runs the scenario and the contract against a server that runs already. |
| `scylla-parity compare <left> <right> [--ignore-meta NAME]` | Prints the differences. The exit code is not 0 when there is a difference. |
| `scylla-parity contract --addr <host:port> --database-url <url>` | Checks the client contract against a server that runs already. |
| `scylla-parity coverage` | Prints the RPCs of the descriptor sets that no scenario step calls. The exit code is not 0 when there is one. The test `every_proto_rpc_is_called_by_a_scenario_step` does the same check. |

`baseline` options:

| Option | Meaning |
|---|---|
| `--runs N` | The number of A runs. The default is 2. |
| `--port P` | The port of the server. Use a port that no other run uses. |
| `--out-dir DIR` | The output folder. The default is `target/parity/<edition>`. |
| `--suffix S` | Adds `S` to the names of the Postgres containers (`scylla-parity-<edition>-<S>-run-N`). Use it to run two baselines at the same time. |
| `--b-server PATH` | A second server (B). The baseline does one more run with B. |
| `--b-env K=V` | An environment variable for B. You can give it more than one time. |
| `--ignore-meta NAME` | Removes the metadata key `NAME` from the comparisons. Use it for an accepted difference. |

The outputs are in `<out-dir>/run-N/` (and `<out-dir>/run-b/`): `scenario.jsonl`,
`config.toml` and `server.log`. `scenario.jsonl` has the scenario steps, then one line
for each contract check.

## Acceptance of a new implementation

An acceptance run compares a new server (B) with the CE server (A):

```
scylla-parity baseline --server <A> --b-server <B> --b-env SCYLLA_STRATA_PROJECTS=1 \
  --port 18100 --out-dir target/parity-harness/acceptance --suffix acc
```

1. The baseline runs A two times and B one time. Each run has a fresh database.
2. It compares A run 1 with A run 2. This is the noise floor. It must show 0
   differences. If not, the harness or the scenario is not stable: correct it first.
3. It compares A run 1 with B. Each difference is a change that a client can see.
4. It prints the contract result of each side.
5. The exit code comes from A only (the A runs, their contracts, and the noise floor).
   The B differences do not change the exit code.

Examine each difference of step 3. A difference is accepted only when it is a known
and wanted change. A known change that does not show in the output is a harness
error: add a step or a check that shows it.

The differences come in groups. One change can move the labels of later steps (for
example, a project that B does not create moves the `<id:N>` and `<ts:N>` numbers).
Read the first difference of a group first.

## Data

The scenario and the contract are data, in `data/`. An edition adds its own files with
`Edition::extend` (see `src/edition.rs`). The EE repository uses this.

A scenario step has these fields:

| Field | Meaning |
|---|---|
| `call` | `package.Service/Rpc`. The client reads the types from the descriptor sets. |
| `req` | The request, in the proto JSON form. `${name}` takes the value of a variable. |
| `auth` | The bearer token, for example `${admin_token}`. No `auth` sends no header. |
| `header` | A raw `authorization` value, for example `bearer ${admin_token}`. It replaces `auth`. |
| `headers` | Other request metadata, as a map. |
| `timeout_ms` | The deadline of the call (`grpc-timeout`). For `drain` and `close`: the wait. |
| `save` | Variables from the response, as JSON pointers (proto field names). |
| `expect` | The expected code. The default is `OK`. Another code is an "unexpected" line. |
| `sql` | A SQL statement on the database of the server, instead of a call. It records no result. |
| `query` | A SQL query. The step records its rows as the body. |
| `sort` | JSON pointers to arrays that the output sorts. `""` is the body. Use it for a list whose order is not stable. |
| `mask` | JSON pointers to values that the output replaces with `<masked>`. |
| `set` | Variables with a fixed value. |
| `open` | Opens a stream with the name `NAME` on `call` (default `scylla.agent.v1.AgentService/Open`). The client sends the messages of `send` first. The upstream stays open. A task keeps each message that comes down. |
| `send` | The first messages of `open`, in the proto JSON form. |
| `drain` | Waits until `count` new messages came on the stream, or 2 s. The body is the new messages. Without `count`, it waits 2 s. Fewer messages than `count` is an "unexpected" line. |
| `close` | Closes the upstream of the stream and waits for its end. The body is the messages that came after the last `drain`. |
| `hold` | Starts a transaction on a dedicated database connection and runs the SQL in it (for example a `SELECT ... FOR UPDATE`). The locks stay until `release`. |
| `release` | `true`: rolls back the transaction of `hold`. |
| `sleep_ms` | Waits, with no call. |

Each call sends `x-request-id: parity-N`, where `N` is the step number.

A contract file has these parts. The parts of all the files (CE, then EE) are merged:
the lists are appended.

| Part | Meaning |
|---|---|
| `setup` | Scenario steps that make the variables of the checks. |
| `unauthenticated.calls` | One call for each authenticated service. The client sends `{}`. |
| `unauthenticated.cases` | The token cases: `name`, `auth` or `header`, `headers`, `timeout_ms`, the expected `code` and `message`. `codes` gives a different code for some calls; the message of these calls is not checked. |
| `load` | A list. Each entry: `call`, `req` (default `{}`), `auth`, `parallel`, `expect` (default `OK`). The client splits the calls over 12 connections. Each result must be `expect`, and the count of results must be `parallel`. An error of the client counts as `HARNESS_ERROR`. |
| `store_failure` | A list. Each entry: `name`, `break_sql`, `repair_sql`, `auth`, `calls` (default: `unauthenticated.calls`), `code` (default `INTERNAL`), `message` (default `Internal server error`). The check breaks the store, does the calls, then repairs the store. |

## Normalization

Each output line keeps the label, the service, the RPC, the code, the message, the
body and `meta`. `meta` has the response metadata and trailers, sorted, without
`content-type`, `date`, `grpc-status`, `grpc-message`, `grpc-accept-encoding`,
`grpc-encoding` and `vary`. `status-details` has the base64 of the status details
when they are not empty.

The output replaces these values:

1. The value of each variable becomes `<name>` (ids, tokens, secrets). When a
   variable gets a new value, the first value keeps `<name>` and the next values get
   `<name#2>`, `<name#3>`. When two variables have the same value, the first name is
   used. A value is replaced only at a token border (the characters before and after
   it are not letters or digits), so a known value inside an id does not break the id.
2. The address of the server becomes `<server>`.
3. An RFC 3339 timestamp becomes `<ts:N>`, also inside a text. `N` is the order of
   its first use. Equal values have the same label. A changed value gets a new label.
4. Each other ULID or UUID becomes `<id:N>`, in the order of its first use. An id that
   the scenario writes in a request (an unknown id) stays as it is.
5. A message with unknown fields gets the key `<unknown_fields>` with their count.

The run also writes an "unexpected" line when a step saves a value from an id field
(a pointer that ends with `/value` or `_id`) and the value is not a lowercase ULID.
Builtin role ids (for example `organization-admin`) are not ULIDs and are not checked.

Calls that are not stable without help:

- `RoleService/ListRoles` with a custom role: the position of the custom role changes
  with its random id. The step sorts `/roles`.
- `JobService/TailJobLogs`: a stream. The client reads it for 500 ms. The line has the
  messages that came in that time and `stream open` or `stream closed`.
- `audit log` (the last step): the server writes the audit rows from a queue, after
  the answer. The step before it waits 1 s. The query groups the rows and counts
  them, and the step sorts them. The ids and the timestamps of the rows are not in
  the query.

Three CE baseline runs on 2026-10-26 showed no unstable timestamp: all the
`<ts:N>` labels were equal. No step masks a timestamp. If a field becomes unstable
(for example `last_active_at` of a session), mask it with `mask`.

## CE values

The expectations come from CE runs (CE commit e1637fe):

- The `authorization` value must start with `Bearer ` (case-sensitive). Another scheme,
  `bearer`, `BEARER` or a bare token gives `UNAUTHENTICATED`
  "Missing or invalid authorization token". `Bearer  T` (two spaces), `Bearer T `
  (a trailing space) and `Bearer ` give `UNAUTHENTICATED` "Invalid or expired token".
- `grpc-timeout: abc` and `x-request-timeout-ms: 0` with a valid token change nothing.
- When the `sessions` table or the `app_tokens` table is not there, each call gives
  `INTERNAL` "Internal server error".
- A `CreateProject` that waits on a row lock and has a deadline of 300 ms gives
  `CANCELLED` "Timeout expired". After the lock is released, the project is not there.
- The agent stream gets the dispatch of a job, and a cancel when its project is deleted.
- CE has no limit on the number of projects of an organization.
- CE checks the permission of `CreateProject` before it looks for the organization:
  an unknown organization gives `PERMISSION_DENIED` to a user with no grant.
- CE sends no `x-request-id` back.

## Limits

- The SQL steps use the CE schema (`sessions`, `password_resets`, `organizations`,
  `audit_log`). A server with a different schema needs different SQL steps.
- The scenario uses the bootstrap admin `admin` / `admin123` of the generated config.
- The client also enforces the deadline of a call: a server that sends no answer in
  time and a server that cancels the call give the same line. The step after the
  deadline shows if the server did the work.
