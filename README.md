# Scylla: core and Community edition

The public repository of Scylla. It holds the core and the `scylla-ce` binary. The Enterprise edition is in a separate private repository (`scylla-ops/scylla-ee`) and adds its features through plugins. The protos are in `scylla-ops/scylla-protos`, a git submodule at `proto`: clone with `git clone --recurse-submodules`, or run `git submodule update --init` in a clone.

Users sign up or sign in with an identity provider, and work in workspaces with their projects and Apps (machine identities, and the agents that run jobs). The API is gRPC and gRPC-Web on one port. Access is zero trust: a short-lived Biscuit token on each request, and a SpiceDB permission check on each operation.

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
| `scylla-domain` | pure types: `Id<T>` and its codec, the permission model, the platform, actors, users and handles, slugs, instance admins, workspaces, Apps and agents, projects and their secrets, pages |
| `scylla-extension` | the contract: `Edition`, `Plugin`, hooks, `Gate` |
| `scylla-proto` | generated code of `proto/`, the submodule `scylla-protos` |
| `scylla-core` | access (Biscuit, SpiceDB), features (accounts, instance admins, sessions, providers, invitations, workspaces, actors, Apps, agents, tokens, projects, secrets, grants, capabilities), mails, logs, the guard against the public dev keys, server, CLI |
| `scylla-testkit` | starts the real server and gate in a test, one helper per core RPC, a capture of the log lines |
| `scylla-ce` | the Community edition binary |
| `scylla-agent` | the agent binary: `join`, `run` and the Windows service, the stream, the order of the nodes, the processes and the containers, the logs with the secrets masked |

## Access

| Step | How |
|---|---|
| A person signs up | `AccountService.SignUp` with an email and a password, then the link of the mail (`VerifyEmail`). The mode `SCYLLA_SIGN_UP_MODE` is `invite_only` by default: the link of an invitation is necessary. The first user is a user of `user create --admin`. |
| A person signs in | With an email or a handle and a password: `AccountService.SignIn`. In a browser with an identity provider: `/auth/<slug>/start`, then the session cookie, or a code on the page `sign-in-error` of the web app. The verified email of an account with a password goes to the page `sign-in-link`, where the password of the account links the identity (`LinkPendingIdentity`). A native client: `TokenService.ExchangeIdToken`. Each sign-in opens a session, and `TokenService.Refresh` gives new tokens. |
| An App signs in | An admin creates the App (or an App creator: the role `app_creator` of the workspace, which then manages its own Apps) in a workspace (`AppService.CreateApp`) and gets a one-time join code (15 minutes). The machine makes its Ed25519 key pair and sends the code and its public key: `TokenService.JoinApp`. Then each time it signs an assertion with its private key: `TokenService.ExchangeAppAssertion`. The token stops at once when its key is revoked (`RevokeAppKey`) or its App is deleted. Or an admin creates an API token (`AppService.CreateAppToken`, 7 to 365 days) and the program exchanges its secret `scya_...`: `TokenService.ExchangeApiToken`. Or a CI job of a CI identity of the App (`AppService.AddCiIdentity`) exchanges the ID token of its job: `TokenService.ExchangeCiToken`. |
| A program calls in JSON | `curl -X POST https://scylla.example.com/api/scylla.projects.v1.ProjectService/ListProjects -H "Authorization: Bearer scya_..." -d '{}'`: the gateway takes the proto3 JSON of a unary method and answers JSON, with the HTTP status of the gRPC code. A token, or the secret of an App or personal token, goes in `Authorization`. |
| An agent connects | An App created with `runner = true` opens `AgentService.OpenStream`, sends a hello with its host, and sends a new token on the stream before the expiry of the old one. `GetApp` and `ListApps` show if it is connected, its host, its last seen time and the number of jobs that it holds. `GetAgentStats` and `ListAgentStats` give the ends of its jobs for each of the last 30 days, and their run times. |
| Each request | `authorization: Bearer <biscuit>`. 10 minutes by default. A holder can attenuate it offline before it gives it to a step. The operation of a request is its gRPC method. |
| Each route | The permission of the route: the RPC name in snake case. An owner of the workspace and an instance admin pass with no SpiceDB call, and a custom role passes with one SQL query (decision 86). Else one SpiceDB call looks for a role on the object or on its workspace. |
| A share | `AccessService.Grant` and `Revoke`: a role (`admin`, `member`, `viewer` in a workspace, `admin`, `editor`, `viewer` on a project) for one actor. An App gets roles only in its own workspace. `ListAccess` lists the owners and the roles of an object, and `CheckPermissions` tells a caller its permissions on an object. `InvitationService` invites an email into a workspace, and lists, revokes and sends again the open invitations. |
| A list | `ListWorkspaces` and `ListProjects` show only what the caller can see (SpiceDB reverse lookups for the roles). `workspace_id` filters the projects. |
| A change of a project | `ProjectService.UpdateProject`: the name and the description, for the editors of the project and the members of its workspace. |
| A pipeline | `PipelineService`: a graph of 1 to 256 nodes (a command or a script, an environment with literals and secrets of the project, a timeout, retries, a work folder, a process or a container). `UpdatePipeline` needs the version that the client read, else `ABORTED`. `RunPipeline` makes a pending job with a snapshot of the nodes. |
| A job | An agent with the role `runner` in its workspace gets the oldest pending job on its stream, with the values of the secrets, while it holds fewer jobs than its capacity. A job with a container goes only to an agent with a container runtime. The agent sends the events and the logs back on the stream. `JobService` reads, cancels, deletes and runs again a job, lists the jobs, and follows the logs with `TailJobLogs`. A job of an agent that went away for 60 seconds ends as orphaned. |
| A trigger | `TriggerService`: a cron schedule (5 fields, UTC) or a webhook, with inputs (literals, or JSON pointers into the body of a webhook) that each node gets. A fire starts a job as the runner of the triggers of the workspace. The webhook route `POST /webhooks/{id}` is public: the HMAC-SHA256 of the body with the secret of the trigger is the credential. |
| A secret | `SecretService` in a project: create, give a new value, delete, list the names in name order. No RPC returns a value. The server encrypts each value with `SCYLLA_SECRET_KEY` and the id of its secret. The admins of a project and of its workspace change the secrets, and its editors and the members of the workspace list them. |
| An account | `ProfileService`: the profile and the handle, a change of email with a link to the new email, the password, the identities, the sessions (with their IP address), and the deletion of the account. |
| The roles | `RoleService`: the catalog of the permissions, the built-in roles, and the custom roles of the instance or of a workspace. `AccessService.Grant` on the platform gives the roles of the whole instance. |
| The users of the instance | `UserAdminService`, for the instance admins and the roles that give its routes: list, read, create, change, disable, delete, reset link, sessions, and the instance admin right. The last active instance admin stays. |

A workspace contains the projects and the Apps, and has a unique slug for the URLs. Its admins rename it, and its owners delete it when it is empty. One user or more own it: `WorkspaceService.AddOwner`, `RemoveOwner` and `ListOwners` change and read the owners, and the last owner stays. Each user has a personal workspace. Postgres holds the containers and their owners (`workspace_id`, `workspace_owners`). SpiceDB holds only the roles that a person gives. A creation writes nothing to SpiceDB.

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
```

Also Docker, Rust 1.94 or later, `just` and `protoc`. `grpcurl` is optional.

## Run

`.env` holds dev values only: the database URL, the SpiceDB key, a dev signing key, the SMTP of Mailpit and a dev key for the secrets. These keys are public: `serve` stops at start if one of them, or the public key of the dev signing key, is in use on a server that is not local (an address that is not a loopback address, a trusted proxy, or a URL that is not on a loopback address), and logs a warning on a local server.

```sh
just run                                   # Postgres, SpiceDB, Mailpit, migrations, schema, server on 127.0.0.1:3000
echo "$PASSWORD" | just user alice@example.com Alice   # an active user with its personal workspace
grpcurl -plaintext -import-path proto -proto scylla/accounts/v1/account_service.proto \
  -d "{\"identifier\":\"alice@example.com\",\"password\":\"$PASSWORD\"}" \
  127.0.0.1:3000 scylla.accounts.v1.AccountService/SignIn
```

The answer has an access token and a refresh token. Mailpit shows the mails of the server at http://127.0.0.1:51080.

The dev setup has no identity provider: the tests use a fake one. To try a real one, add it with `provider add-oidc` or `provider add-github`, then open `http://127.0.0.1:3000/auth/<slug>/start` in a browser.

`scylla-ce --help` lists the commands and each setting with its environment variable and default.

| Command | What it does |
|---|---|
| `serve` | serves gRPC, gRPC-Web and the browser routes of the sign-in (the default) |
| `migrate` | applies the migrations and writes the SpiceDB schema, then stops |
| `user create --email <email> --name <name> [--handle <handle>] [--password-stdin] [--admin]` | creates an active user with its personal workspace, and prints its id; `--admin` makes it an instance admin |
| `user admin grant <id>`, `user admin revoke <id>` | gives or takes back the instance admin right; the last active admin stays |
| `provider add-oidc --slug <slug> --name <name> --issuer <url> --client-id <id> [--client-secret-stdin] [--allow-sign-up]` | adds or changes an OIDC provider |
| `provider add-github --slug <slug> --name <name> --client-id <id> [--allow-sign-up]` | adds or changes a GitHub provider; the client secret comes from stdin |
| `provider list`, `provider disable --slug <slug>` | lists the providers, turns one off |
| `secret keygen` | prints a new key for `SCYLLA_SECRET_KEY` |
| `token keygen` | prints a new signing key pair |
| `token create --actor <id>` | signs a token, for a first admin or an emergency |
| `token revoke <token>` | revokes a token and each token made from it |

## Run an agent

An admin creates an App with `runner = true` (`AppService.CreateApp`) and gives it the role `runner` of the workspace, or of some projects (`AccessService.Grant`). The server gives an install script that downloads the agent, checks its SHA-256, joins the App, and starts the agent as a service:

```sh
# Linux (systemd, user scylla-agent) or macOS (launchd, user of the command)
curl -fsSL https://scylla.example.com/install.sh | sh -s -- <join code> --capacity 2
```

```powershell
# Windows, PowerShell as administrator: the service ScyllaAgent
& ([scriptblock]::Create((irm https://scylla.example.com/install.ps1))) -JoinCode <join code> -Capacity 2
```

```sh
# A container: the first start joins, the volume keeps the file of the agent
docker run -d --name scylla-agent -e SCYLLA_URL=https://scylla.example.com -e SCYLLA_JOIN_CODE=<join code> -v scylla-agent:/var/lib/scylla-agent ghcr.io/scylla-ops/scylla-agent
```

The scripts take `--container-runtime` (`-ContainerRuntime`) and `--no-service` (`-NoService`), and download from the release of the version of the server, or from `SCYLLA_AGENT_DOWNLOADS_URL` (on the server, or on the machine for one install). The release workflow (`.github/workflows/release.yml`, at each tag `v*`) builds the archives, `SHA256SUMS` and the image. By hand, on the machine of the agent:

```sh
scylla-agent --config /var/lib/scylla-agent/agent.json join https://scylla.example.com <join code>
scylla-agent --config /var/lib/scylla-agent/agent.json run --capacity 2 --container-runtime docker
```

On Windows, in PowerShell, the default folder is `%LOCALAPPDATA%\scylla-agent`, and a script step needs `sh` or `bash` in the `PATH` (for example the ones of Git for Windows):

```powershell
scylla-agent join https://scylla.example.com <join code>
scylla-agent run --capacity 2
```

`join` makes the key pair and writes the file (mode 600 on Unix) with the private key; the key never leaves the machine. `run` keeps the stream open, renews its token one minute before its expiry, and connects again after an end (1 to 60 seconds of wait). It stops for good when the server refuses the agent (a revoked key, a deleted App, an App that runs no job). The settings are also environment values: `SCYLLA_AGENT_CONFIG`, `SCYLLA_AGENT_WORKSPACE_ROOT`, `SCYLLA_AGENT_CAPACITY`, `SCYLLA_AGENT_CONTAINER_RUNTIME`, `SCYLLA_AGENT_KEEP_WORKSPACE`. With `SCYLLA_URL` and `SCYLLA_JOIN_CODE`, `run` joins first when the file does not exist. `run --service` is the Windows service; its logs are `agent.<date>.log` next to the file.

## Logs

The server and the commands write their logs to stderr.

| Setting | Default | What it does |
|---|---|---|
| `SCYLLA_LOG_FORMAT` (`--log-format`) | `auto` | `pretty` for a person, `json` for a machine. `auto` is pretty when stderr is a terminal, else JSON. |
| `RUST_LOG` | `info`, and `warn` for `sqlx`, `hyper`, `h2`, `tonic`, `tower`, `tower_http` | Replaces the default filter. |
| `NO_COLOR` | not set | Removes the colors of the pretty format. A pretty line has colors only in a terminal. |

Each call has one line: its result, its route, its duration, its actor and its IP address. A refusal or an error adds a second line with the cause. A business event has its own line with a tag. A CORS preflight has no line.

```text
19:04:22.113  AUTH      account.created                       user=..3c9e41ec  email=l***@example.com
19:04:22.114  MAIL      verify-email -> l***@example.com      sent
19:04:22.114  OK        AccountService.SignUp                 302ms  anonymous         127.0.0.1
19:04:22.741  UNAUTH    AccountService.SignIn                 285ms  anonymous         127.0.0.1
                        invalid identifier or password
19:04:24.217  DENIED    WorkspaceService.GetWorkspace          22ms  user ..7d02fba7   127.0.0.1
                        permission=get_workspace  object=workspace:..b51e2ae9
```

A JSON line has `ts`, `level`, `event` (`rpc` or the name of a business event), `rpc`, `code`, `ms`, `actor`, `ip`, `request_id`, and the fields of the event. Each answer gives its request id in the header `x-request-id`. `DECISIONS.md` (59) lists the business events. A log line never has a password, a token, a cookie, the value of a secret, the body or the link of a mail, or a full email.

## Deploy

1. Store `SCYLLA_TOKEN_PRIVATE_KEY`, `SCYLLA_SPICEDB_KEY` and `SCYLLA_SECRET_KEY` in a secret store. Make the keys with `token keygen` and `secret keygen`. A key of a `.env` file stops the start. With no `SCYLLA_SECRET_KEY`, the server starts but refuses the secrets of the projects (`SECRET_KEY_MISSING`).
2. Run SpiceDB with a Postgres datastore, and set `SCYLLA_SPICEDB_ENDPOINT`.
3. Set `SCYLLA_API_URL` (the public URL of the server), `SCYLLA_APP_URL` (the web app), `SCYLLA_SMTP_URL` and `SCYLLA_MAIL_FROM`, and choose `SCYLLA_SIGN_UP_MODE`.
4. Run `scylla-ce migrate` one time, as a job. Add the first instance admin with `user create --admin`, and the identity providers with `provider add-oidc` or `provider add-github`.
5. Start one server with `scylla-ce serve`: the open streams of the agents are in its memory. A server stops at start if a migration is missing. `SCYLLA_LOG_RETENTION_DAYS` (30) sets the life of the logs of the jobs. With `SCYLLA_WEB_DIR`, the server also serves the built web app on its address (one domain: set `SCYLLA_APP_URL` to the URL of the server); a path that is not a file gives `index.html`.
6. For TLS, set `SCYLLA_TLS_CERT` and `SCYLLA_TLS_KEY`. For browsers, set `SCYLLA_CORS_ORIGINS`: a cookie refresh needs one of these origins. Behind one proxy that adds the client address to `X-Forwarded-For`, set `SCYLLA_TRUST_FORWARDED_HEADERS=true`: a session then keeps the last address of this header, not the address of the proxy.
7. Probe `grpc.health.v1.Health/Check`. Stop with SIGTERM: the server ends the running requests. With no terminal, the logs are JSON lines on stderr.
8. To rotate the signing key: set a new key and a new `SCYLLA_TOKEN_KEY_ID`, and keep the old public key in `SCYLLA_TOKEN_PUBLIC_KEYS` for one token lifetime. Never keep the public key of the dev signing key: the server does not start with it.

## Import the old product

One time, into a base with migrations and no user:

```sh
SCYLLA_IMPORT_FROM=postgres://user:password@old-host/scylla SCYLLA_OLD_SECRET_KEY=<old master_key> scylla-ce import
```

The command prints what it imported and each row that it left, with the reason. The users sign in with their old passwords; their sessions do not come over. The Apps, the agents, the jobs, the logs and the triggers do not come over: an agent joins again with a new key pair.

## Tests

```sh
just test                      # Postgres, the two SpiceDBs and Mailpit must run: just db-up
```

The tests use `spicedb serve-testing` on port 55052: each test gets an empty SpiceDB store.

| Tests | Where |
|---|---|
| Workspaces, projects and their permissions, tokens, Apps and agents, gRPC-Web, CORS, TLS, health | `scylla-ce/tests/api/` |
| Pipelines: the nodes and their graph, the path of a bad value, the version of a change, each role, the snapshot of a run, the secrets of a run, the delete | `scylla-ce/tests/api/projects/pipelines.rs` |
| Jobs: the cancel, the rerun, the delete, the lists newest first, who can call them | `scylla-ce/tests/api/projects/jobs.rs` |
| Agent: the join, the order of the nodes, a script with a secret masked in the logs, a retry, the skip after a failure, a cancel that stops the process and its children and deletes the work folder, on Unix and Windows, the name of the machine on the key, the join at a first start | `scylla-agent/tests/agent.rs` |
| Import of the old product: its real schema, the users and their passwords, a disabled user, a user with no email, the workspaces and their owners, the roles, the project ids, the secrets sealed again, the pipelines and a pipeline with a cycle, a second import | `scylla-ce/tests/api/platform/import.rs` |
| Web app: a file, a route of the web app gives `index.html`, the API on the same address | `scylla-ce/tests/api/platform/web.rs` |
| Triggers: the schedules, the inputs, a fire now as the runner, the signature of a webhook, a second delivery, a body with no value, a ping, a stopped trigger, the rate limit, the roles, the key of the secrets | `scylla-ce/tests/api/projects/triggers.rs` |
| Jobs on the agents: the dispatch with the secrets, the events, the logs and their tail, the role and the capacity, the containers, the cancel, an agent of some projects, the jobs of a disabled agent, the number of jobs of each agent, the stats of the jobs of each agent and who reads them | `scylla-ce/tests/api/projects/runs.rs`, `scopes.rs`, `disabled.rs`, `held.rs`, `stats.rs` |
| Apps: the join code and its one use, the exchange of an assertion, each refused assertion, the keys, their names, their last use and their revocation, the delete, the disable and the enable, who can create, read and change them | `scylla-ce/tests/api/identity/apps.rs`, `disable.rs`, `keys.rs` |
| The install scripts of the agent: the URL, the version and the downloads of the server in `/install.sh` and `/install.ps1` | `scylla-ce/tests/api/identity/install.rs` |
| API tokens of the Apps: the secret one time and its hint, the exchange, the revocation and the expiry that stop the short tokens at once, a disabled or deleted App, the routes and their checks | `scylla-ce/tests/api/identity/app_tokens.rs` |
| The limits of the API tokens: the read-only, deploy and full presets, a custom list of methods, a list of projects, the check of new limits | `scylla-ce/tests/api/identity/token_limits.rs` |
| App creators: the role `app_creator` creates Apps and manages its own only, and `CheckPermissions` tells `create_app` | `scylla-ce/tests/api/identity/app_creators.rs` |
| Personal tokens: the user as actor, no token from a token, the switch of the instance at once, the switch of a workspace on that workspace only | `scylla-ce/tests/api/identity/personal_tokens.rs` |
| CI identities: a GitHub job of the rule gets a token of its App, another repository, ref, audience or App gets none, a removed rule and a disabled App, the claims of GitLab, the routes and their checks | `scylla-ce/tests/api/identity/ci_identities.rs` |
| HTTP/JSON gateway: a method in proto3 JSON, the errors with their HTTP status and their fields, an unknown or streaming method, the secret of an App or personal token sent directly and its revocation at once | `scylla-ce/tests/api/platform/json.rs` |
| Streams of the agents: the hello and the host, the presence, only an agent, the renewal of the token, the end at the expiry, a second stream, a revoked key, a deleted App | `scylla-ce/tests/api/identity/agents.rs` |
| Projects: creation, reads, the list and its pages, deletion | `scylla-ce/tests/api/projects/lifecycle.rs` |
| Change of a project: the name and the description, their limits (100 and 101, 1024 and 1025), one line for a name, an empty description, each role | `scylla-ce/tests/api/projects/updates.rs` |
| Secrets: the metadata, no value in an answer, the form of a name and one secret for each name, the size of a value (65536 and 65537 bytes), a new value, a delete, the pages in name order, the delete of a project | `scylla-ce/tests/api/projects/secrets.rs` |
| Access to the secrets: each role on each route, an unknown project or secret, `CheckPermissions`, a server with no key | `scylla-ce/tests/api/projects/secret_access.rs` |
| Permission model: roles in a workspace and on one project, an extension, a new definition | `scylla-ce/tests/api/tenancy/authz.rs` |
| Ownership: a role in a workspace goes down, an App with a role, no SpiceDB for an owner | `scylla-ce/tests/api/tenancy/ownership.rs` |
| Owners of a workspace: add, remove, list, only an owner changes them, the last owner stays, two removals at the same time | `scylla-ce/tests/api/tenancy/owners.rs` |
| Shares: grant and revoke, each role, only the admins give roles, the visible list, an App only in its workspace | `scylla-ce/tests/api/tenancy/grants.rs` |
| Slugs: the form, the reserved names, one workspace for each slug, the slug of a name and its suffix, the slug of a handle, `GetWorkspace` with a slug, `UpdateWorkspace` | `scylla-ce/tests/api/tenancy/slugs.rs` |
| Deletion of a workspace: owners only, an empty workspace only, its invitations and roles go, a project creation, a new owner or an invitation at the same time | `scylla-ce/tests/api/tenancy/deletion.rs` |
| Permissions of a caller: owner, admin, member, viewer, another user, instance admin, a project, the platform, unknown names | `scylla-ce/tests/api/tenancy/checks.rs` |
| Roles: the catalog, the built-in roles, a custom role of the instance or of a workspace, escalation, a used name, a role in use, a change that is live at once | `scylla-ce/tests/api/tenancy/roles.rs`, `role_changes.rs` |
| The limit of the workspaces: the default, the personal workspace, an instance admin, the instance settings, a delete, two creations at the same time | `scylla-ce/tests/api/tenancy/workspace_limit.rs` |
| Roles of the instance: `Grant` and `Revoke` on the platform, the last admin, what these roles reach, `ListUsers` with the roles, the workspaces and the last sign-in | `scylla-ce/tests/api/tenancy/platform_roles.rs` |
| List of the access: owners and roles of a workspace, roles of a project, the pages, who can read it | `scylla-ce/tests/api/tenancy/access_lists.rs` |
| Accounts: sign-up and its link, the modes, the length of a password (8 and 128), the same answer for each refusal, the lock and 50 failures, the reset, the change of a password | `scylla-ce/tests/api/identity/accounts.rs` |
| Handles: the limits and the characters, one user for each handle, the sign-in with a handle | `scylla-ce/tests/api/identity/handles.rs` |
| Profile: the display name, the sign-out of the other sessions, the change of email with the current password and its link, the notice to the old email, the old links stop | `scylla-ce/tests/api/identity/profile.rs` |
| Mails: text and HTML bodies, the escape, the token in the fragment of each link, one reset link and one email change link each minute, one email change link each minute to one email | `scylla-ce/tests/api/identity/mails.rs` |
| Deletion of an account: the empty personal workspace, the last owner of a workspace with content, a shared workspace, a workspace that the user left during the deletion, the roles, the last admin | `scylla-ce/tests/api/identity/deletion.rs` |
| Instance admins: only they pass each route of the console, grant and revoke, the last active admin, two admins that disable each other at the same time | `scylla-ce/tests/api/identity/admins.rs` |
| Console of the users: list and filter, create, change, a new email stops the old links, disable, delete, reset link, sessions of a user | `scylla-ce/tests/api/identity/console.rs` |
| Sessions: rotation and reuse, sign-out, list and revoke, a disabled user, the cookie and its origin, the IP address with and without a trusted proxy | `scylla-ce/tests/api/identity/sessions.rs` |
| Identity providers: browser sign-in with OIDC and GitHub, return address, no automatic link, same browser, invitations; `ExchangeIdToken` | `scylla-ce/tests/api/identity/sign_in.rs`, `sign_in_errors.rs`, `sign_in_link.rs`, `sign_in_link_refusals.rs`, `sign_in_mails.rs`, `oidc.rs` |
| Invitations: the role of the link, admins only, the email of the invitation, sign-up with a link | `scylla-ce/tests/api/tenancy/invitations.rs` |
| Open invitations: the list, the revoke, the resend and its cooldown, one open invitation for each email | `scylla-ce/tests/api/tenancy/open_invitations.rs` |
| An acceptance at the same time as a revoke or a resend | `scylla-ce/tests/api/tenancy/invitation_races.rs` |
| Route rule: no check, a group check, the own check, an error | `scylla-ce/tests/api/platform/routes.rs` |
| Logs: the request id of each answer, one line with its permission and object for a refusal, the reason of a refusal of a rule, no line for a preflight, no secret and no full email in the lines of a sign-up, a sign-in, a reset and the secrets | `scylla-ce/tests/api/platform/logs.rs` |
| Route permission names, records of the gate | `scylla-extension/src/operation.rs` |
| Schema of the parts, typed values | `scylla-domain/src/authz/tests.rs` |
| `authz!`: schema text, checks of the names, errors | `scylla-macros/src/tests.rs` |
| Hook rollback, order and kind filter | `scylla-core/src/features/projects/tests.rs` |
| The value of a secret opens only with the id of its secret, and the database has no value in clear | `scylla-core/src/features/projects/secrets/tests.rs` |
| Config and command line, the guard against the public dev keys | `scylla-core/src/config.rs`, `scylla-core/src/cli/commands.rs`, `scylla-core/src/infra/dev_keys.rs` |
| The `access.refused` event of a refusal | `scylla-extension/src/tests.rs` |
| The pretty and JSON lines: alignment, second line, colors, fields, masks | `scylla-core/src/infra/logs/tests.rs` |
| Slugs and the tokens of the list of the access | `scylla-domain/src/tenancy/slugs.rs`, `grants.rs` |
| The names and the values of the secrets, a value hidden in `Debug` | `scylla-domain/src/projects/secrets.rs` |
| Random tokens, PKCE, the encryption of the secrets with and without associated data, cookies, the client of a session, the layout of the mails | `scylla-core/src/infra/secret.rs`, `scylla-core/src/features/identity/sessions/cookie.rs` and `client.rs`, `scylla-core/src/features/identity/accounts/letter.rs` |

## Releases for the Enterprise edition

The Enterprise repository takes `scylla-core`, `scylla-domain`, `scylla-extension`, `scylla-proto` and `scylla-testkit` from one Git tag (`scylla-macros` comes with `scylla-domain`). A change to these items can stop its build at the next tag: `Edition`, `Plugin`, `Gate`, the role registry `Grants`, `Mailer`, `Operation`, the hook traits and contexts, `scylla_core::Error`, `parse`, `cli`, `Config`, `ServeOptions`, the domain types, `scylla_domain::authz` and `authz!`, the names of the definitions `actor`, `platform`, `workspace` and `project`, `TestApp`, and the tables `actors`, `users`, `workspaces` and `projects`. `scylla_core::Error` has `Conflict` (`ABORTED`), `NewJob` and `Job` have `trigger_id`, `Config` and `ServeOptions` have `web_dir`, `tokio` has the features `process`, `io-util` and `fs`, `tonic` has `tls-webpki-roots`, the workspace has the dependency `nix`, `CreateContext` and `DeleteContext` have `subject`, `object` and `role` (with `with_member` and `with_object`), `AccountContext` has `hooks`, the core runs the hooks of the kinds `member` and `invitation`, the `tower-http` dependency has the feature `fs`, the core has the dependencies `croner` and `hmac`, `Config` has `log_retention`, `ServeOptions` has `log_retention`, `DomainError` is `Clone` and has new variants, and the core adds the definitions `app` and the kinds `pipeline` and `job` for the hooks. `Gate::issue` takes a `Binding` (none, a session, or the key of an App), and `Principal::new` takes the binding and the expiry of the token. The `Gate` trait has `delete_subject`, `subjects`, `roles` and `permissions`, `EditionBuilder::grants` takes `AccessRoutes` and a required search for the workspace, `AccessError::denied` takes the principal and logs the event `access.refused`, `Config` has `log_format`, `scylla_core::infra::logs` gives the log layer and the masks, the testkit has `Logs`, a `Mail` has an HTML body, and the details of a `Rejection` are texts. The table `workspaces` has a required `slug`. The field `TestApp.sealer` is an `Option`, `NewProject` has a `description`, the table `projects` has a column `description`, a `Name` refuses a control character, and `DomainError` has new variants.

## Known limits

- On Windows, the agent stops a step at once, with no grace: a job object has no stop signal.
- The server is one instance: the open streams of the agents are in memory.
- No route reads the value of a secret yet: the dispatch of a job will use `secrets::service::reveal`.
