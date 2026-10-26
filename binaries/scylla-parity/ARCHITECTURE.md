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
| `--b-env K=V` | An environment variable for B. You can give it more than one time. B must also serve each key of `SCYLLA_STRATA_SLICES`. |
| `--ignore-meta NAME` | Removes the metadata key `NAME` from the comparisons. Use it for an accepted difference. |

The outputs are in `<out-dir>/run-N/` (and `<out-dir>/run-b/`): `scenario.jsonl`,
`config.toml` and `server.log`. `scenario.jsonl` has the scenario steps, then one line
for each contract check.

The generated config has `[webhook] public_base_url = "http://<address>"`. The server
starts with `RUST_LOG=warn,scylla_strata=info,scylla_core=info` and `NO_COLOR=1`, so
that the `log` steps can read the password reset links. `run` reads the log file of
`--server-log PATH`.

## Acceptance of a new implementation

An acceptance run compares a new server (B) with the CE server (A):

```
scylla-parity baseline --server <A> --b-server <B> \
  --port 18100 --out-dir target/parity-harness/acceptance --suffix acc
```

1. The baseline runs A two times and B one time. Each run has a fresh database.
2. It compares A run 1 with A run 2. This is the noise floor. It must show 0
   differences. If not, the harness or the scenario is not stable: correct it first.
3. It compares A run 1 with B. Each difference is a change that a client can see.
4. It prints the contract result of each side.
5. It reads the log of B (the mount check). B must log "strata serves the slice" for each
   slice of `Edition::strata_slices`, "the engine runs the worker" with `engine="strata"` for
   each worker of `Edition::strata_workers`, and "the engine runs the webhook ingress" with
   `engine="strata"`. A missing line, or one of these lines with `engine="ce"`, fails the run.
6. The exit code comes from every check: the A runs, their contracts, the noise floor, the B
   run, its contract, the mount check and the compare of A run 1 with B.

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
| `open` | Opens a stream with the name `NAME` on `call` (default `scylla.agent.v1.AgentService/Open`). The client sends the messages of `send` first. The upstream of a client stream stays open. For a server stream (for example `JobService/TailJobLogs`), the first message of `send` is the request and the client closes the upstream. A task keeps each message that comes down. An open that the server refuses records the code and the message of the refusal. |
| `send` | The first messages of `open`, or the messages of `push`, in the proto JSON form. |
| `push` | Sends the messages of `send` on the open stream `NAME` (for example a status or a log line of an agent). |
| `drain` | Waits until `count` new messages came on the stream, the stream ended, or 2 s (`timeout_ms` changes the wait). The body is the new messages. The message is `open`, or `closed CODE MESSAGE` when the stream ended. The code is `OK`. Fewer messages than `count` is an "unexpected" line. |
| `close` | Closes the upstream of the stream and waits for its end. The body is the messages that came after the last `drain`. The message is the same as for `drain`. |
| `until` | With `call` or `query`: the step repeats until each JSON pointer of the map has its value, or until `poll_ms` (default 2000) ends. The record is the last answer. `"<present>"` and `"<absent>"` test if a value is there. The key `code` tests the gRPC code. `every_ms` (default 100) is the time between two tries. |
| `http` | An HTTP/1.1 request to the address of the server: `method` (default `POST`), `path`, `headers`, `body` (text). `sign` is the name of a variable with a webhook secret: the request gets `X-Scylla-Signature-256: sha256=<hex HMAC-SHA256 of the body>`. The record has the service `http`, the path as the RPC, the status number as the code and the response body as the message. `expect` is the status number, for example `"202"`. |
| `log` | Waits up to 2 s (`timeout_ms`) for a line of `server.log` that the server wrote after the start of the step before. The line must have the `target` and the `message`. `keep` is a list of `key=value` fields of the line for the body. `save` is a map of variables: each one takes the text after a prefix (for example `#token=`) up to the next space; the body gets its length. No line gives the code `NO_LOG`. |
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

- A step that waits for a side effect: the wait is a `query` step with `until` on the
  CE tables (`jobs`, `job_logs`, `agents`, `pipeline_triggers`). Then one call reads the
  state. Do not poll with an RPC: each RPC writes an audit row, and the last step counts
  the audit rows.
- Agent log lines: the server writes one audit row for each group of lines that it reads
  at the same time. Do not send two log lines of one job one after the other without a
  wait. Send a status frame between them, or wait for the first line (`job_logs`, a
  `drain` of a tail).
- Log lines and status frames that the steps compare have fixed timestamps (whole
  microseconds), because the database keeps microseconds.

## Side effect blocks

The steps that start with `bN` check side effects of the jobs, agents, auth and triggers
slices. Each block makes its own project, pipeline and agent in `acme`, and deletes them
at its end, so the steps after the block do not change. The expected codes come from CE.

| Block | What it shows |
|---|---|
| `b1` | Status and log frames: a started job is `RUNNING`; the log lines are stored; `JobCompleted` gives the next pending job to the agent at once; late frames of a finished job change nothing; `CancelJob` sends a cancel. |
| `b2` | Live tail: the snapshot, a new line, a line sent again (no copy), an older line, a tail on a node, the end of the tail on `JobCompleted`, `CancelJob` and `DeleteJob`. A tail as `bob` is `PERMISSION_DENIED`. A tail of an unknown job: `NOT_FOUND` with a node, a closed stream without a node. |
| `b3` | Cancel frames: `CancelJob` and `DeleteJob` send a cancel; a failure after a cancel changes nothing; a finished job gets no cancel and `CancelJob` is `FAILED_PRECONDITION`. |
| `b4` | Release and reconcile: a closed stream puts its pending job back in the pool; a new stream gets it; a closed stream keeps a running job; a hello with an unknown job gets a cancel; a hello without the running job orphans it; a second stream of one app replaces the first one (`ALREADY_EXISTS`). |
| `b5` | Agent views: the open of a stream sets `last_seen` before a hello; an app that is not an agent can open a stream; the host of the hello, `connected`, `in_flight`, the stats. A stream ends when its app is deactivated, its secret is disabled or revoked, or the agent is deleted. |
| `b6` | The webhook route: accepted, duplicate, bad signature, ping, a NUL in an input, a disabled trigger, a deleted trigger. The job has the webhook origin and the `GIT_COMMIT` input. |
| `b7` | Reset links in `server.log`: no link for an inactive user, one link for an active user, no second link in the cooldown, no link for an unknown email. The link resets the password once and ends the old sessions. |
| `b11` | Auth: an agent stream with a user token, the client of a session (`user-agent`, the address), tokens that are not live, `RevokeToken` two times, a bearer on a public call does not move `last_active_at`. The contract also checks `TailJobLogs`. |
| `b10` | Reconcile opens the tail of a running job that the agent keeps. |
| `b12` | `FireTriggerNow` with a missing secret, the secrets and the access of a new agent, the reserved name `trigger-runner`. |
| `b8` | The cron trigger is enabled again. A trigger `* * * * *` makes a cron job in less than 75 s (this block makes the run about one minute longer). |

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
- An agent stream with a user token gives `PERMISSION_DENIED` "only an agent reports
  its own state". `TailJobLogs` with `{}` gives `INVALID_ARGUMENT` "missing job_id".
- A session keeps the `user-agent` of the login (`parity-ua/1.0 tonic/0.14.6`) and the
  address `127.0.0.1`: CE does not trust `x-forwarded-for` and `x-real-ip` by default.
- `FireTriggerNow` with a secret that is not in the project gives `NOT_FOUND`, and the
  trigger shows a failed fire. `CreateAgent` with the name `trigger-runner` gives
  `INVALID_ARGUMENT` "App name 'trigger-runner' is reserved".
- The webhook route answers `202 accepted`, `200 duplicate`, `401 invalid signature`,
  `200 pong`, `422 invalid payload` and `404 not found`.

## Mutant runs

A mutant is a one-line change in `crates/scylla-strata` that removes a side effect. B is
built with the mutant, then the baseline runs with `--runs 1`. Each mutant must make B
differ from A. The step numbers are those of the scenario with 525 steps (2026-10-26).
No mutant stays in the tree: revert the file after the build.

| Mutant | Change | First step that shows it |
|---|---|---|
| M1 | `record_job_status` without `job_tails.open` | 274 `b2 t1 gets l1 and l2` (the tail ends at once) |
| M2 | `record_job_status` without `job_stops.wake` | 246 `b1 dispatch of job_b at once` (no dispatch) |
| M3 | the agent `Reader` without `release_agent_jobs` | 396 `b5 get the disconnected agent` (`in_flight` stays 1) |
| M4 | `reconcile_agent_jobs` without the cancel of the stale ids | 364 `b4 cancel of the unknown job` (no cancel) |
| M5 | `cancel_job` without `job_stops.stop` | 255 `b1 cancel of job_b` (no cancel) |
| M6 | `request_password_reset` without `issue_later` | 441 `b7 link for erin` (`NO_LOG`) |
| M7 | `create_trigger` without `schedule_next` | 171 `create cron trigger` (no `next_fire_at`) |
| M8 | `Open` without the first `touch_agent` | 383 `b5 the open alone sets last_seen` |
| M9 | `login` with `SessionClient::default()` | 26 `alice sessions` (no `user_agent` and no `ip_address`) |
| M10 | the Apps slot gets a second, new `InMemoryAgentRegistry` in `host.rs` | 138 `runner-3 gets the job` (no dispatch) |

M8 did not fail before the steps `b5 agent opens with no hello` and
`b5 the open alone sets last_seen`: the hello also sets `last_seen`.

## Limits

- The SQL steps use the CE schema (`sessions`, `password_resets`, `organizations`,
  `audit_log`, `jobs`, `job_logs`, `agents`, `pipeline_triggers`). A server with a
  different schema needs different SQL steps.
- The scenario uses the bootstrap admin `admin` / `admin123` of the generated config.
- The client also enforces the deadline of a call: a server that sends no answer in
  time and a server that cancels the call give the same line. The step after the
  deadline shows if the server did the work.
