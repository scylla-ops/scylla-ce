# Scylla: core and Community edition

The public repository of Scylla. It holds the core and the `scylla-ce` binary. The Enterprise edition is in a separate repository (`../ee` in this POC) and adds its features through plugins.

Users sign up or sign in with an identity provider, and work in workspaces with their projects and agents. The API is gRPC and gRPC-Web on one port. Access is zero trust: a short-lived Biscuit token on each request, and a SpiceDB permission check on each operation.

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
| `scylla-domain` | pure types: `Id<T>` and its codec, the permission model, actors, users, workspaces, agents, projects, pages |
| `scylla-extension` | the contract: `Edition`, `Plugin`, hooks, `Gate` |
| `scylla-proto` | generated code of `proto/` |
| `scylla-core` | access (Biscuit, SpiceDB), features (accounts, sessions, providers, invitations, workspaces, actors, agents, tokens, projects, grants, capabilities), mails, server, CLI |
| `scylla-testkit` | starts the real server and gate in a test, one helper per core RPC |
| `scylla-ce` | the Community edition binary |

## Access

| Step | How |
|---|---|
| A person signs up | `AccountService.SignUp` with an email and a password, then the link of the mail (`VerifyEmail`). The mode `SCYLLA_SIGN_UP_MODE` is `invite_only` by default: the link of an invitation is necessary. The first user is a user of `user create`. |
| A person signs in | With a password: `AccountService.SignIn`. In a browser with an identity provider: `/auth/<slug>/start`, then the session cookie. A native client: `TokenService.ExchangeIdToken`. Each sign-in opens a session, and `TokenService.Refresh` gives new tokens. |
| An agent signs in | `AgentService.RegisterAgent` in a workspace (by an admin, with the agent public key), then each time an assertion signed with the agent private key: `TokenService.ExchangeAgentAssertion`. |
| Each request | `authorization: Bearer <biscuit>`. 10 minutes by default. A holder can attenuate it offline before it gives it to a step. The operation of a request is its gRPC method. |
| Each route | The permission of the route: the RPC name in snake case. An owner of the workspace passes with no SpiceDB call. Else one SpiceDB call looks for a role on the object or on its workspace. |
| A share | `AccessService.Grant` and `Revoke`: a role (`admin`, `member`, `viewer` in a workspace, `admin`, `editor`, `viewer` on a project) for one actor. `InvitationService` invites an email into a workspace. |
| A list | `ListWorkspaces` and `ListProjects` show only what the caller can see (SpiceDB reverse lookups for the roles). `workspace_id` filters the projects. |

A workspace contains the projects and the agents. One user or more own it: `WorkspaceService.AddOwner`, `RemoveOwner` and `ListOwners` change and read the owners, and the last owner stays. Each user has a personal workspace. Postgres holds the containers and their owners (`workspace_id`, `workspace_owners`). SpiceDB holds only the roles that a person gives. A creation writes nothing to SpiceDB.

The permission model is Rust code. `authz!` declares a SpiceDB definition next to its type, and the compiler checks each name and each id. A route permission uses a group, and a role gives a group:

```rust
authz! {
    /// The roles on one project.
    definition project for Project {
        /// An actor that can do everything on this project.
        relation admin: Actor;
        /// The group of the routes that delete the project.
        permission delete = admin;
        /// The route `DeleteProject`, on one project.
        permission delete_project = delete;
    }

    /// The routes of the projects on their workspace.
    extend Workspace as ProjectHolder {
        /// The route `DeleteProject`, on each project of this workspace.
        permission delete_project = manage;
    }
}

gate.authorize(&caller, Project::DELETE_PROJECT.on(project_id).within(workspace)).await?;
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

`.env` holds dev values only: the database URL, the SpiceDB key, a dev signing key, the SMTP of Mailpit and a dev key for the client secrets.

```sh
just run                                   # Postgres, SpiceDB, Mailpit, migrations, schema, server on 127.0.0.1:3000
echo "$PASSWORD" | just user alice@example.com Alice   # an active user with its personal workspace
grpcurl -plaintext -import-path proto -proto scylla/accounts/v1/account_service.proto \
  -d "{\"email\":\"alice@example.com\",\"password\":\"$PASSWORD\"}" \
  127.0.0.1:3000 scylla.accounts.v1.AccountService/SignIn
```

The answer has an access token and a refresh token. Mailpit shows the mails of the server at http://127.0.0.1:58025.

The dev setup has no identity provider: the tests use a fake one. To try a real one, add it with `provider add-oidc` or `provider add-github`, then open `http://127.0.0.1:3000/auth/<slug>/start` in a browser.

`scylla-ce --help` lists the commands and each setting with its environment variable and default.

| Command | What it does |
|---|---|
| `serve` | serves gRPC, gRPC-Web and the browser routes of the sign-in (the default) |
| `migrate` | applies the migrations and writes the SpiceDB schema, then stops |
| `user create --email <email> --name <name> [--password-stdin]` | creates an active user with its personal workspace, and prints its id |
| `provider add-oidc --slug <slug> --name <name> --issuer <url> --client-id <id> [--client-secret-stdin] [--allow-sign-up]` | adds or changes an OIDC provider |
| `provider add-github --slug <slug> --name <name> --client-id <id> [--allow-sign-up]` | adds or changes a GitHub provider; the client secret comes from stdin |
| `provider list`, `provider disable --slug <slug>` | lists the providers, turns one off |
| `secret keygen` | prints a new key for `SCYLLA_SECRET_KEY` |
| `token keygen` | prints a new signing key pair |
| `token create --actor <id>` | signs a token, for a first admin or an emergency |
| `token revoke <token>` | revokes a token and each token made from it |

## Deploy

1. Store `SCYLLA_TOKEN_PRIVATE_KEY`, `SCYLLA_SPICEDB_KEY` and `SCYLLA_SECRET_KEY` in a secret store. Make the keys with `token keygen` and `secret keygen`.
2. Run SpiceDB with a Postgres datastore, and set `SCYLLA_SPICEDB_ENDPOINT`.
3. Set `SCYLLA_API_URL` (the public URL of the server), `SCYLLA_APP_URL` (the web app), `SCYLLA_SMTP_URL` and `SCYLLA_MAIL_FROM`, and choose `SCYLLA_SIGN_UP_MODE`.
4. Run `scylla-ce migrate` one time, as a job. Add the first admin with `user create`, and the identity providers with `provider add-oidc` or `provider add-github`.
5. Start the servers with `scylla-ce serve`. A server stops at start if a migration is missing.
6. For TLS, set `SCYLLA_TLS_CERT` and `SCYLLA_TLS_KEY`. For browsers, set `SCYLLA_CORS_ORIGINS`: a cookie refresh needs one of these origins.
7. Probe `grpc.health.v1.Health/Check`. Stop with SIGTERM: the server ends the running requests.
8. To rotate the signing key: set a new key and a new `SCYLLA_TOKEN_KEY_ID`, and keep the old public key in `SCYLLA_TOKEN_PUBLIC_KEYS` for one token lifetime.

## Tests

```sh
just test                      # Postgres, the two SpiceDBs and Mailpit must run: just db-up
```

The tests use `spicedb serve-testing` on port 55052: each test gets an empty SpiceDB store.

| Tests | Where |
|---|---|
| Workspaces, projects and their permissions, tokens, agents, gRPC-Web, CORS, TLS, health | `scylla-ce/tests/api/` |
| Permission model: roles in a workspace and on one project, an extension, a new definition | `scylla-ce/tests/api/authz.rs` |
| Ownership: a role in a workspace goes down, an agent with a role, no SpiceDB for an owner | `scylla-ce/tests/api/ownership.rs` |
| Owners of a workspace: add, remove, list, only an owner changes them, the last owner stays, two removals at the same time | `scylla-ce/tests/api/owners.rs` |
| Shares: grant and revoke, each role, only the admins give roles, the visible list | `scylla-ce/tests/api/grants.rs` |
| Accounts: sign-up and its link, the modes, the same answer for each refusal, the lock, the reset, the change of a password | `scylla-ce/tests/api/accounts.rs` |
| Sessions: rotation and reuse, sign-out, list and revoke, a disabled user, the cookie and its origin | `scylla-ce/tests/api/sessions.rs` |
| Identity providers: browser sign-in with OIDC and GitHub, return address, no automatic link, same browser, invitations; `ExchangeIdToken` | `scylla-ce/tests/api/sign_in.rs`, `oidc.rs` |
| Invitations: the role of the link, admins only, the email of the invitation, sign-up with a link | `scylla-ce/tests/api/invitations.rs` |
| Route rule: no check, a group check, the own check, an error | `scylla-ce/tests/api/routes.rs` |
| Route permission names, records of the gate | `scylla-extension/src/operation.rs` |
| Schema of the parts, typed values | `scylla-domain/src/authz/tests.rs` |
| `authz!`: schema text, checks of the names, errors | `scylla-macros/src/tests.rs` |
| Hook rollback, order and kind filter | `scylla-core/src/features/projects/tests.rs` |
| Config and command line | `scylla-core/src/config.rs`, `scylla-core/src/cli.rs` |
| Random tokens, PKCE, the encryption of the secrets, cookies | `scylla-core/src/secret.rs`, `scylla-core/src/features/sessions/cookie.rs` |

## Releases for the Enterprise edition

The Enterprise repository takes `scylla-core`, `scylla-domain`, `scylla-extension`, `scylla-proto` and `scylla-testkit` from one Git tag (`scylla-macros` comes with `scylla-domain`). A change to these items can stop its build at the next tag: `Edition`, `Plugin`, `Gate`, the role registry `Grants`, `Mailer`, `Operation`, the hook traits and contexts, `scylla_core::Error`, `parse`, `cli`, `Config`, `ServeOptions`, the domain types, `scylla_domain::authz` and `authz!`, the names of the definitions `actor`, `workspace` and `project`, `TestApp`, and the tables `actors`, `users`, `workspaces` and `projects`.

## Known limits

- The pipelines are not there yet. An agent token can already be attenuated for each step.
