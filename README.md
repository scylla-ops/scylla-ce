# Scylla

Distributed CI/CD platform.

## Architecture

| Service                | Ports          | Description                                                       |
|------------------------|----------------|-------------------------------------------------------------------|
| `scylla-ce`            | `8080`         | One binary: web UI, gRPC API, gRPC-Web, webhook ingress, and in-process job dispatch |
| `scylla-agent`         | —              | Worker installed per machine; registered as an "App" in the UI, run out-of-band |
| `postgres`             | `5432`         | Primary datastore (PostgreSQL 18)                                 |

Two binaries ship: `scylla-ce` (the control plane, Community Edition) and `scylla-agent` (remote workers). Agents connect to the control plane over a persistent worker stream, there is no message broker.

Everything the browser and the agents talk to lives on **one port**. The control plane serves the compiled-in web UI, terminates gRPC-Web for the browser, speaks native gRPC to the agents, and accepts inbound webhooks at `/webhooks/{trigger_id}` — all on `8080`. Because the UI is served from the same origin as the API, the bundle uses relative URLs: the published image carries no baked-in hostname and works unchanged in any deployment.

The workspace is a stack of library crates under `crates/` with the two binaries under `binaries/`: `scylla-ce` (the control plane) and `scylla-agent` (the worker). See [Workspace layout](#workspace-layout) below.

## Workspace layout

```
crates/
  scylla-extension/   the edition boundary: the action pipeline every write goes through (depends on scylla-domain only)
  scylla-domain/      the shared kernel: domain model, JobEvent (no I/O, no crypto)
  scylla-proto/       the Rust bindings and their conversions; the .proto files are the scylla-protos submodule in proto/
  scylla-auth/        the access model: RBAC ports and types, the Cedar adapter
  scylla-core/        use cases and their ports, gRPC + HTTP surfaces, server config, in-memory adapters
  scylla-db/          the Postgres adapters, the pool, the embedded migrations
  scylla-server/      the composition root: the Server builder and the shared cli
binaries/
  scylla-ce/          the Community Edition binary: a main.rs and config/*.toml
  scylla-agent/       the worker binary (depends on scylla-domain + scylla-proto only)
web/                  the scylla-web submodule: the web UI, compiled into scylla-ce through scylla-core
migrations/           the SQL schema, embedded by scylla-db
```

The `.proto` files are in
[scylla-ops/scylla-protos](https://github.com/scylla-ops/scylla-protos) and the
web UI is in [scylla-ops/scylla-web](https://github.com/scylla-ops/scylla-web).
Both are git submodules, so clone with `--recurse-submodules`, or run
`git submodule update --init --recursive`.

Dependencies point one way, bottom to top:

```
scylla-domain <- scylla-proto     <- scylla-core <- scylla-db <- scylla-server <- scylla-ce
scylla-domain <- scylla-auth      <- scylla-core
scylla-domain <- scylla-extension <- scylla-core
```

`scylla-core` holds the ports declared in `scylla-core` and `scylla-auth` as trait objects (`Arc<dyn Port>`); `scylla-db` implements them; `scylla-server` is the only crate that names the concrete implementations side by side. The binaries are a `main.rs` each: parse the command line, load the configuration, open the pool with `scylla_db::init_db`, then `Server::new(config, db)`, the edition's contributions as builder methods, `serve()`.

A WebAssembly plugin runtime is planned as `crates/scylla-wasm/`; it does not exist yet.

## Editions and the extension mechanism

This repository is the open source core and ships the Community Edition binary, `scylla-ce`. A separate, private repository builds an Enterprise binary on top of it. The dependency is strictly one way: the private repository depends on this one by a pinned git tag, and nothing here knows it exists.

The seam between the two is `crates/scylla-extension`, the action pipeline. Every write in the core is a command (`CreateProject { organization_id, name, description }`) that `Actions::run` moves through three typed stages: `Authorize` (the access check, run by the core for every action: the action declares `Access::Public`, `Access::Authenticated` or the permissions it requires), `Prepare` (the use case reads and builds a staged value, it does not write) and `Persist` (the store writes inside `Prepared::commit`). Every read is a query (`GetProject { id }`) that the same `Actions::run` moves through `Authorize` and `Fetch`; the trait the action implements (`Command` or `Query`) decides what follows the access check. Around each stage the `Hooks` registry runs six positions, in this order:

```
Policy -> Gate<S> -> Around [ Wrap<S> [ Run ] ] -> Listener<S> -> Observer
 veto      veto      control   control            side effect     record
 every     one       every     one                one             every
```

`Policy`, `Around` and `Observer` are erased: registered on a stage kind, they fire for every command, including commands that do not exist yet, and see the caller, the `Access` (its permissions through `access().permissions()`) and the action id. `Gate<S>`, `Wrap<S>` and `Listener<S>` are typed: registered on one stage of one command, they see the exact phase. A veto is a `DomainError` (`quota_exceeded`, `business_rule`) and maps to gRPC like any other. The crate depends on `scylla-domain` and nothing else, so an edition builds against a pinned git tag without the access model, the database or the gRPC stack.

An edition bundles its hooks in a type that implements `Extension` and registers it once:

```rust
impl Extension for MeteredQuota {
    fn register(self: &Arc<Self>, hooks: &mut Hooks) {
        hooks
            .policy(StageKind::Prepare, self.clone())   // veto a create over the plan's limit
            .observe(StageKind::Persist, self.clone()); // count what was stored
    }
}

Server::new(config, db).extension(&Arc::new(MeteredQuota::new(plans))).serve()
```

The Community Edition registers nothing: the pipeline runs the access check, the stages and no hook. A use case is a struct that holds its ports and implements `Run<Prepare<C>>` and `Run<Persist<C>>` per command (`commands.rs`) and `Run<Fetch<Q>>` per query (`queries.rs`), one block per action; it has no method of its own. The gRPC handler is the adapter: each RPC is one `grpc::adapter::run` call (`run_public`, as `Anonymous`, for the services without the bearer interceptor and for HTTP routes), which parses the request into its command or query (`grpc::convert::Parse`, implemented in the aggregate's mapper), runs the engine and maps the error. The project use case and its handler are the reference; see `crates/scylla-extension/ARCHITECTURE.md` for the positions, the decision table and the failure semantics.

The same builder takes what else an edition contributes, usually bundled as a `scylla_server::Feature` (its hooks, its migrations on the shared pool, its services): `authenticated_grpc_service` / `grpc_service` (its own gRPC services on the same listener, behind the core's bearer-token interceptor or not), `file_descriptor_set` (so reflection lists them) and `http_routes`. A feature's `install` step receives a `Context` with the pool, the core's `Actions` and its authorization ports, so its use cases run through the same pipeline and policies as the core's. A private edition therefore adds whole features without touching this repository.

## Prerequisites

You only need Docker. Everything else (Rust, Node.js, pnpm) runs inside containers.

- **Docker** (>= 24) with **Docker Compose** v2
- **[just](https://github.com/casey/just)** (optional, shortcuts for `docker compose`)

Check versions:

```sh
docker --version
docker compose version
```

## Quick start

Prebuilt images are published for both `linux/amd64` and `linux/arm64` — Docker pulls the right one for your host automatically.

One command pulls the prebuilt images and starts the stack (control plane, PostgreSQL). Agents are added afterward from the UI — see below:

> [!WARNING]
> Coming from an earlier beta? Wipe the previous stack first with `just clean` — it removes the old containers, volumes, and locally-built images, which are not compatible across betas. This release also drops the separate `scylla-frontend` service, and `docker compose down` alone would leave it behind as an orphan.

```sh
git clone https://github.com/scylla-ops/scylla.git
cd scylla
just up
# or, without just:
docker compose pull
docker compose up -d
```

The first start creates the `admin` user. The `[bootstrap]` section of the
config sets its username, password and email. At each start, the server finds
this account by its email and makes sure that it is a system administrator.

Open **http://localhost:8080/** and sign in:

- username: `admin` (or the email `admin@example.com`)
- password: `admin123`

## Developing

The stack above is enough to run Scylla. To work on it:

```sh
just db-up                                        # Postgres alone
cargo run -p scylla-ce -- \
    --config binaries/scylla-ce/config/local.toml --no-ui
```

`--no-ui` drops the UI routes, so the binary serves only the API.
(`config/local.toml` already sets `[ui].enabled = false`.) To work on the UI,
run the dev server of [scylla-web](https://github.com/scylla-ops/scylla-web)
against `:8080`; `local.toml` allows its origin through CORS.

To run the real thing natively, build the UI first — a release `cargo build`
embeds whatever is in `web/dist`. The UI and the protos are git submodules, so
get them first (or clone with `--recurse-submodules`):

```sh
git submodule update --init --recursive
just ui-build
cargo build --release -p scylla-ce
```

## TLS

The control plane speaks plain HTTP by default, on the assumption that a reverse
proxy or ingress terminates TLS in front of it. To terminate it in the binary
instead, point `[server.tls]` at a PEM chain and key:

```toml
[server]
address = "0.0.0.0:8443"

[server.tls]
cert = "/etc/scylla/tls/fullchain.pem"
key  = "/etc/scylla/tls/privkey.pem"
```

Both HTTP/2 and HTTP/1.1 are advertised over ALPN, so browsers, gRPC clients and
plain HTTP/1.1 webhook senders all connect to the same socket.

## Behind a reverse proxy

Each session records the IP address of the client that signed in. The user
sees it in the list of its sessions. By default, the server uses the peer
address of the connection. Behind a reverse proxy, that is the address of the
proxy. To record the address of the client, set
`[server].trust_forwarded_headers`:

```toml
[server]
address = "0.0.0.0:8080"
trust_forwarded_headers = true
```

The server then uses the first entry of `x-forwarded-for`, else `x-real-ip`. A
call without these headers keeps the peer address. The default is `false`.

Set it to `true` only behind a reverse proxy that sets these headers and
replaces the values that the client sends. A client can write these headers,
so without such a proxy a client can record any address.

## Password reset

A reset link sets a new password. It opens `/reset-password` in the web UI. It
is good for one use and expires after one hour. A user asks for a link with
`AuthService.RequestPasswordReset`. An administrator makes a link for a user
with `UserService.SendPasswordReset`.

The Community Edition sends no mail. It writes each reset link in the server
log, at INFO, with the user id and the username. Look for the message
`password reset link`, for example with `just logs scylla-ce`, and give the
link to the user. The log line does not show the email.

Set `[server].public_url` to the origin that people open, without a trailing
slash:

```toml
[server]
address = "0.0.0.0:8080"
public_url = "https://scylla.example.com"
```

The link is then `https://scylla.example.com/reset-password#token=...`. If you
do not set `public_url`, the link is the relative path
`/reset-password#token=...` and the server writes a warning when it starts.
The shipped configs set it: `http://127.0.0.1:8080` in `local.toml` and
`http://localhost:8080` in `docker.toml`.

## Common commands

| `just`        | `docker compose`                  | What it does                                  |
|---------------|-----------------------------------|-----------------------------------------------|
| `just up`     | `docker compose pull && up -d`    | Pull and start (or refresh) the beta stack    |
| `just down`   | `docker compose down`             | Stop the stack                                |
| `just clean`  | `docker compose down -v --rmi local --remove-orphans` | Stop and wipe volumes + local images |
| `just logs [svc]` | `docker compose logs -f [svc]` | Follow logs (all services or one)            |
| `just status` | `docker compose ps`               | Show running containers                       |

Run `just --list` to see every recipe.

## Troubleshooting

**Port already in use.** Another process holds `8080` or `5432`. Stop it or change the host port in `docker-compose.yaml`.

**`scylla-ce` fails to connect to PostgreSQL.** Ensure `postgres` is `healthy` via `just status` (or `docker compose ps`). If it's stuck, run `just clean` to reset the volume and try again.

**The web UI shows gRPC errors.** The UI and the API share an origin, so there is no CORS step to get wrong; check that `scylla-ce` is `healthy` (`just status`) and read its logs. `curl http://localhost:8080/healthz` should answer `ok`.

**Agent not picking up jobs.** Agents run out-of-band (not in this compose stack). Check the agent's own logs and confirm it can reach the control plane at its `--control-plane-url` with a valid `--app-id` / `--app-secret`. In the UI the app shows as connected once its worker stream is open.

> Still stuck? Open a post in the `help` Discord channel with:
>  - steps to reproduce
>  - `docker compose logs` output for the affected service
>  - `docker compose ps` snapshot

## Optional: local development

The Docker workflow above is self-contained: **Node.js and Rust are not required** to run Scylla. The section below is only for contributors who want to build the crates outside Docker.

### Building crates locally

Requires the Rust toolchain. Standard Cargo workflow:

```sh
git submodule update --init
just hooks
cargo build
cargo test
```

To build the Docker images from source instead of pulling them:

```sh
just local
```

## Further reading

- [Glossary](GLOSSARY.md), every Scylla-specific term, grouped by topic.
- Releasing images: see [RELEASING.md](RELEASING.md). `just release <version>` builds and pushes the multi-arch images; `just --list` shows the individual recipes.

## License

Apache License 2.0, see [LICENSE](./LICENSE).
