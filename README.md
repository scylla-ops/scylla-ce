# Scylla: core and Community edition

The public repository of Scylla. It holds the core and the `scylla-ce` binary. The Enterprise edition is in a separate repository (`../ee` in this POC) and adds its features through plugins.

Users and agents own and create projects. The API is gRPC and gRPC-Web on one port. Access is zero trust: a short-lived Biscuit token on each request, and a SpiceDB permission check on each operation.

- `AGENT.md`: how to write code here (structure, access, comments, tests, protos, SQL).
- `DECISIONS.md`: why the code has its shape.

## Crates

```
scylla-ce ──▶ scylla-core ──▶ scylla-extension ──▶ scylla-domain ──▶ scylla-macros
                  │
                  └──────▶ scylla-proto
scylla-testkit (tests only) ──▶ scylla-core
```

| Crate | Contents |
|---|---|
| `scylla-macros` | `authz!`: the permission model in the SpiceDB schema language |
| `scylla-domain` | pure types: `Id<T>` and its codec, the permission model, actors, users, agents, projects, pages |
| `scylla-extension` | the contract: `Edition`, `Plugin`, hooks, `Gate` |
| `scylla-proto` | generated code of `proto/` |
| `scylla-core` | access (Biscuit, SpiceDB), features (actors, users, agents, tokens, projects, capabilities), server, CLI |
| `scylla-testkit` | starts the real server and gate in a test, one helper per core RPC |
| `scylla-ce` | the Community edition binary |

## Access

| Step | How |
|---|---|
| A person signs in | OIDC ID token of the identity provider, then `TokenService.ExchangeIdToken`. The first sign-in creates the user. |
| An agent signs in | `AgentService.RegisterAgent` (by its user, with the agent public key), then each time an assertion signed with the agent private key: `TokenService.ExchangeAgentAssertion`. |
| Each request | `authorization: Bearer <biscuit>`. 10 minutes by default. A holder can attenuate it offline before it gives it to a step. The operation of a request is its gRPC method. |
| Each route | The permission of the route: the RPC name in snake case. The owner of the object, or an actor above it, passes with no SpiceDB call. Else one SpiceDB call looks for a grant on the object or on its owners. |

Postgres holds who owns what (`owner_id`, and `actors.owner_id` for the user of an agent). SpiceDB holds only the grants that a person gives: delegates, roles on an owner, roles on one object. A creation writes nothing to SpiceDB.

The permission model is Rust code. `authz!` declares a SpiceDB definition next to its type, and the compiler checks each name and each id. A route permission uses a group (`act_for`, `delete`, `edit`, `view`), and a role gives a group:

```rust
authz! {
    /// The grants on one project.
    definition project for Project {
        /// An actor that can do everything on this project.
        relation admin: Actor;
        /// The group of the routes that delete the project.
        permission delete = admin + admin->act_for;
        /// The route `DeleteProject`, on one project.
        permission delete_project = delete;
    }

    /// The routes of the projects on their owner.
    extend Actor as ProjectOwner {
        /// The route `DeleteProject`, on each project of this owner.
        permission delete_project = act_for;
    }
}

gate.authorize(&caller, Project::DELETE_PROJECT.on(project_id).owned_by(owner)).await?;
app.grant(&[Project::ADMIN.of(project_id, carol)]).await;
```

A route that checks a group, or no permission, gets `PERMISSION_DENIED`. An edition adds a definition the same way, or adds to a core definition with `authz! { extend Project as ProjectAudit { ... } }`.

## Tools

Use the latest version of each tool:

```sh
cargo install sqlx-cli --locked --features sqlx-toml
npm install -g @bufbuild/buf
```

Also Docker, Rust 1.94 or later, `just` and `protoc`. `grpcurl` is optional.

## Run

`.env` holds dev values only: the database URL, the SpiceDB key and a dev signing key.

```sh
just run                       # Postgres, SpiceDB, migrations, schema, server on 127.0.0.1:3000
ACTOR=$(just actor user)       # creates an actor
TOKEN=$(just token $ACTOR)     # signs a token for it
grpcurl -plaintext -import-path proto -proto scylla/projects/v1/project_service.proto \
  -H "authorization: Bearer $TOKEN" -d '{"name":"alpha"}' \
  127.0.0.1:3000 scylla.projects.v1.ProjectService/CreateProject
```

To sign in as a person, start the dev identity provider and uncomment the OIDC lines of `.env`:

```sh
just keycloak                  # Keycloak on 127.0.0.1:58080, user alice, password alice
curl -s -d grant_type=password -d client_id=scylla -d username=alice -d password=alice \
  -d scope=openid http://127.0.0.1:58080/realms/scylla/protocol/openid-connect/token
```

Then send the `id_token` to `scylla.auth.v1.TokenService/ExchangeIdToken`.

`scylla-ce --help` lists the commands and each setting with its environment variable and default.

| Command | What it does |
|---|---|
| `serve` | serves gRPC and gRPC-Web (the default) |
| `migrate` | applies the migrations and writes the SpiceDB schema, then stops |
| `actor create --kind <kind>` | creates an actor and prints its id |
| `token keygen` | prints a new signing key pair |
| `token create --actor <id>` | signs a token, for a first admin or an emergency |
| `token revoke <token>` | revokes a token and each token made from it |

## Deploy

1. Store `SCYLLA_TOKEN_PRIVATE_KEY` and `SCYLLA_SPICEDB_KEY` in a secret store. Make the key with `token keygen`.
2. Run SpiceDB with a Postgres datastore, and set `SCYLLA_SPICEDB_ENDPOINT`.
3. Set `SCYLLA_OIDC_ISSUER` and `SCYLLA_OIDC_AUDIENCE` for the identity provider of the company.
4. Run `scylla-ce migrate` one time, as a job.
5. Start the servers with `scylla-ce serve`. A server stops at start if a migration is missing.
6. For TLS, set `SCYLLA_TLS_CERT` and `SCYLLA_TLS_KEY`. For browsers, set `SCYLLA_CORS_ORIGINS`.
7. Probe `grpc.health.v1.Health/Check`. Stop with SIGTERM: the server ends the running requests.
8. To rotate the signing key: set a new key and a new `SCYLLA_TOKEN_KEY_ID`, and keep the old public key in `SCYLLA_TOKEN_PUBLIC_KEYS` for one token lifetime.

## Tests

```sh
just test                      # Postgres and the two SpiceDBs must run: just db-up
```

The tests use `spicedb serve-testing` on port 55052: each test gets an empty SpiceDB store.

| Tests | Where |
|---|---|
| Projects and their permissions, tokens, agents, OIDC, gRPC-Web, CORS, TLS, health | `scylla-ce/tests/api/` |
| Permission model: roles on an owner and on one project, an extension, a new definition | `scylla-ce/tests/api/authz.rs` |
| Ownership: an owner acts on what its agents own, a grant goes down, no SpiceDB for an owner | `scylla-ce/tests/api/ownership.rs` |
| Route rule: no check, a group check, the own check, an error | `scylla-ce/tests/api/routes.rs` |
| Route permission names, records of the gate | `scylla-extension/src/operation.rs` |
| Schema of the parts, typed values | `scylla-domain/src/authz/tests.rs` |
| `authz!`: schema text, checks of the names, errors | `scylla-macros/src/tests.rs` |
| Hook rollback, order and kind filter | `scylla-core/src/features/projects/tests.rs` |
| Config and command line | `scylla-core/src/config.rs`, `scylla-core/src/cli.rs` |

## Releases for the Enterprise edition

The Enterprise repository takes `scylla-core`, `scylla-domain`, `scylla-extension`, `scylla-proto` and `scylla-testkit` from one Git tag (`scylla-macros` comes with `scylla-domain`). A change to these items can stop its build at the next tag: `Edition`, `Plugin`, `Gate`, `Operation`, the hook traits and contexts, `scylla_core::Error`, `parse`, `cli`, `Config`, `ServeOptions`, the domain types, `scylla_domain::authz` and `authz!`, the names of the definitions `actor` and `project`, `TestApp`, and the tables `actors` and `projects`.

## Known limits

- The pipelines are not there yet. An agent token can already be attenuated for each step.
