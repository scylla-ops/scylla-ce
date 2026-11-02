# Scylla: core and Community edition

The public repository of Scylla. It holds the core and the `scylla-ce` binary. The Enterprise edition is in a separate repository (`../ee` in this POC) and adds its features through plugins.

The domain is small on purpose: actors own and create projects. The API is gRPC.

- `AGENT.md`: how to write code here (structure, comments, tests, protos, SQL).
- `DECISIONS.md`: why the code has its shape.

## Crates

```
scylla-ce ──▶ scylla-core ──▶ scylla-extension ──▶ scylla-domain
                  │
                  └──────▶ scylla-proto
scylla-testkit (tests only) ──▶ scylla-core
```

| Crate | Contents |
|---|---|
| `scylla-domain` | pure types: `Id<T>` and its codec, actors, API key ids, projects, pages |
| `scylla-extension` | the contract: `Edition`, `Plugin`, hooks, authenticators |
| `scylla-proto` | generated code of `proto/` |
| `scylla-core` | core features (actors, API keys, projects, capabilities), server, command line, config, errors |
| `scylla-testkit` | starts the real server in a test, one helper per core RPC |
| `scylla-ce` | the Community edition binary |

## Tools

Use the latest version of each tool:

```sh
cargo install sqlx-cli --locked --features sqlx-toml
npm install -g @bufbuild/buf
```

Also Docker, Rust 1.94 or later, `just` and `protoc`. `grpcurl` is optional.

## Run

```sh
just run                       # Postgres, migrations, server on 127.0.0.1:3000
ACTOR=$(just actor service)    # creates an actor of the kind "service"
just key $ACTOR                # prints the key id and the key, one time
grpcurl -plaintext -import-path proto -proto scylla/projects/v1/project_service.proto \
  -H "authorization: Bearer <key>" -d '{"name":"alpha"}' \
  127.0.0.1:3000 scylla.projects.v1.ProjectService/CreateProject
```

`just run` also trusts the `x-scylla-actor` header, for quick tests. Never set `SCYLLA_TRUST_ACTOR_HEADER=true` in production.

`scylla-ce --help` lists the commands and each setting with its environment variable and default.

| Command | What it does |
|---|---|
| `serve` | serves the gRPC API (the default) |
| `migrate` | applies the migrations, then stops |
| `actor create --kind <kind>` | creates an actor and prints its id |
| `key create --actor <id>` | creates an API key and prints it one time |
| `key revoke <key id>` | revokes an API key |

## Deploy

1. Run `scylla-ce migrate` one time, as a job.
2. Start the servers with `scylla-ce serve`. A server stops at start if a migration is missing.
3. For TLS, set `SCYLLA_TLS_CERT` and `SCYLLA_TLS_KEY` (PEM files). Without them, put a TLS proxy in front.
4. Probe `grpc.health.v1.Health/Check`. Stop with SIGTERM: the server ends the running requests.

## Tests

```sh
just test
```

| Tests | Where |
|---|---|
| gRPC API: projects, API keys, capabilities, health, TLS | `scylla-ce/tests/api/` |
| Hook rollback, order and kind filter | `scylla-core/src/features/projects/tests.rs` |
| Config and command line | `scylla-core/src/config.rs`, `scylla-core/src/cli.rs` |

## Releases for the Enterprise edition

The Enterprise repository takes `scylla-core`, `scylla-domain`, `scylla-extension`, `scylla-proto` and `scylla-testkit` from one Git tag. A change to these items can stop its build at the next tag: `Edition`, `Plugin`, the hook traits and contexts, `Authenticator`, `scylla_core::Error`, `parse`, `cli`, `Config`, `ServeOptions`, the domain types, `TestApp`, and the tables `actors`, `api_keys` and `projects`.

## Known limits

- Persons have no sign-in flow yet (for example OIDC). An API key works for each actor kind.
