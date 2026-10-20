# Releasing images

Scylla ships two container images per release. This is the whole procedure:
what gets built, where it lands, what the tags mean, and how to undo a bad
publish.

```sh
just release-setup          # once per machine, plus `docker login`
just release 0.4.0-beta     # build and push everything
```

## What ships, and where

| Image | Built from | Contents |
|---|---|---|
| `scylla-ce` | `Dockerfile`, stage `scylla-ce` | the web UI, gRPC API and gRPC-Web, job dispatch, cron scheduler, webhook ingress |
| `scylla-agent` | `Dockerfile`, stage `scylla-agent` | the worker installed per machine |

Both come out of the same workspace `Dockerfile`; the bake target picks the
final stage. Only the control plane's branch builds the web UI, so the agent
image never pays for the Node toolchain.

They publish to **Docker Hub** under the `godlyjaaaaj` account, as
`godlyjaaaaj/<image>:<tag>`. Override with the `DOCKER_USER` environment
variable.

Each is a **multi-arch manifest** covering `linux/amd64` and `linux/arm64`, so
`docker pull` resolves correctly on an Apple Silicon laptop and an x86 server
alike. A single-arch publish is easy to do by accident and only fails on
someone *else's* machine, so `just release-verify` exists to check.

### The images carry no deployment-specific configuration

The web UI is compiled into the control-plane binary and served from the same
origin as the API, so the bundle addresses it with relative URLs. There is no
API hostname baked into an artifact and therefore no per-deployment build: one
published image works wherever it is run.

## What the tags mean

| Tag | Meaning |
|---|---|
| `0.4.0`, `0.4.0-beta` | An exact release. Immutable — never republish over one. |
| `latest` | What `just up` pulls. Moves to each new release unless you opt out. |
| `beta` | Optional moving channel for pre-releases. |

Image tags drop the `v` that git tags carry: git `v0.4.0-beta` publishes images
tagged `0.4.0-beta`.

**`latest` moves with every release by default.** Scylla is pre-1.0 and every
release so far has been a beta — `latest` has pointed at the newest beta since
0.1.0 — so gating the channel on a stable/pre-release distinction the project
does not have yet would simply freeze it.

Opt out with `LATEST=false` when publishing something that must not become the
default pull:

```sh
LATEST=false just release 0.3.1     # republish an older line, leave latest alone
```

Rolling `latest` back to an earlier version is a retag, not a build. See below.

## Before the first release on a machine

```sh
docker login -u godlyjaaaaj    # once; credentials are cached
just release-setup             # creates the `scylla-builder` buildx builder
```

`release-setup` is idempotent and `just release` runs it for you. `docker
login` is the one step nothing can do on your behalf; without it the build runs
to completion and then fails at the push.

## Releasing

**Before you start**, confirm these yourself — the recipes are deliberately
thin and check none of them:

- [ ] everything is landed on `main`, and `git status` is clean — the images
      record an `org.opencontainers.image.revision` label, which otherwise
      points at a commit that does not describe them
- [ ] `.sqlx/` is present and current (`just db-prepare`) — the backend builds
      with `SQLX_OFFLINE=true` and dies deep inside the Rust compile without it
- [ ] `git submodule status --recursive` shows no `+` or `-` prefix, and the
      pinned `scylla-web` and `scylla-protos` commits are pushed: the build
      needs both, and nobody else can build a tag whose pins are not on
      GitHub
- [ ] you are logged in to Docker Hub

Then:

1. **Tag and push the tag:**

   ```sh
   git tag -a v0.4.0-beta -m "Scylla v0.4.0-beta"
   git push origin refs/tags/v0.4.0-beta
   ```

   Push with the full `refs/tags/` path. A bare `git push origin v0.4.0-beta`
   is ambiguous whenever a branch shares the name, and git refuses with
   `src refspec … matches more than one`.

2. **Check the plan, then publish:**

   ```sh
   just release-plan 0.4.0-beta    # resolved tags and args, builds nothing
   just release 0.4.0-beta
   ```

   Read the plan's tags before building: they are the whole `latest` decision.

3. **Verify, then write the GitHub release** against the tag, in the shape of
   the previous ones: a short intro, `## Highlights`, `## Breaking changes`
   when there are any, `## What's in this release`, `## Quick start`,
   `## Feedback`, then the generated `## What's Changed`. Mark pre-releases as
   such.

### How long it takes

On a laptop the non-native architecture builds under QEMU emulation, which is
slow: budget **45 to 90 minutes** for a full release on Apple Silicon. The web
UI is pinned to the builder's architecture and takes about two minutes.

Both images build in **one** `docker buildx bake` invocation, concurrently
against a single BuildKit session. That is the point of the bake file: the
control plane and the agent share their entire `cargo chef` dependency layer,
so baking them together cooks it once instead of twice.

To build a single image without paying for the others, call bake directly:

```sh
VERSION=0.4.0-beta docker buildx bake -f docker-bake.hcl --push ce
VERSION=0.4.0-beta docker buildx bake -f docker-bake.hcl --push agent
```

## Verifying

```sh
just release-verify 0.4.0-beta
```

It prints each published manifest. Check that every entry lists both
`linux/amd64` and `linux/arm64`; an `unknown/unknown` line is the attestation
manifest and is expected.

## Moving a channel, and rolling back

`release-promote` repoints a channel tag at an already-published version. It
copies the manifest list rather than rebuilding, so it is instant and keeps
every architecture:

```sh
just release-promote 0.4.0-beta beta    # newest beta, for testers
just release-promote 0.4.0 latest       # a bad 0.5.0 shipped — go back
```

Rolling back *is* this: point `latest` at the previous good version. Never
delete or overwrite a version tag — someone is pinning it, and a mutated
version tag makes "works on my machine" unfalsifiable.

It takes the channel name explicitly and asks no questions, so it also serves
any channel the bake file does not manage — `beta`, `edge`, a customer pin.

## Command reference

| Command | Does |
|---|---|
| `just release-setup` | create the buildx builder (idempotent) |
| `just release-plan <version>` | print the resolved plan, build nothing |
| `just release <version>` | build and push both images |
| `just release-verify <version>` | print the published manifests |
| `just release-promote <version> <tag>` | move a channel tag, no rebuild |

Environment: `DOCKER_USER`, `BUILDER` (top of the `justfile`), plus the bake
variables below — most usefully `LATEST=false`. Platforms live in the bake
file's `PLATFORMS`, not the justfile.

## How the build is defined

`docker-bake.hcl` holds the build itself — targets, platforms, tags, labels,
build args and cache — and the `just` recipes are one-line pass-throughs to it.
Anything about *how* an image is built belongs in the bake file, not the
justfile.

Its variables: `VERSION`, `LATEST`, `DOCKER_USER`, `PLATFORMS`, `GIT_SHA`,
`CACHE`. Targets: `ce`, `agent`. One group, `release`, holding both.

Driving bake directly is fine, but **always pass `-f docker-bake.hcl`**:

```sh
docker buildx bake -f docker-bake.hcl --print release
```

Without `-f`, bake also loads `docker-compose.yaml` and turns its services into
extra targets named after them — single-arch, untagged, and easy to build by
mistake.

### Shared build cache

`CACHE` is empty by default, so builds rely on the local builder's cache. Point
it at a registry ref to share layers between machines:

```sh
CACHE=godlyjaaaaj/scylla-buildcache just release 0.4.0
```

The control plane and the agent deliberately share one cache ref: they have
every layer in common up to `cargo build -p`, and that is what makes the second
image nearly free.

## Troubleshooting

**`denied: requested access to the resource is denied`.** Not logged in, or
`DOCKER_USER` is not your account. Run `docker login -u godlyjaaaaj`.

**The Rust build fails on a `query!` macro.** `.sqlx/` is stale or missing. Run
`just db-prepare` and commit the result.

**The build is slow.** That is emulation, not a bug. Building
`PLATFORMS=linux/arm64` alone on Apple Silicon is fast, but the result is
**not** a release: it leaves x86 users with nothing to pull.

**`docker buildx bake` builds the wrong thing.** You forgot `-f
docker-bake.hcl` and got a compose service.

## Design notes

The release used to be a loop of `docker buildx build` calls in the justfile,
documented by one line in the README, with two sharp edges: `VERSION` defaulted
to `latest`, so a bare `just release` tagged the images `latest` twice and
published no version at all; and every release moved `latest`, betas included.

Moving the build into `docker-bake.hcl` fixed both by construction, and bought
the concurrency and the shared dependency layer along the way. The `latest`
rule is derived from the version inside the bake file rather than guarded by a
wrapper, which is why the justfile has no validation code in it — the same rule
`docker/metadata-action` calls `latest=auto`, and the same file CI would use
through `docker/bake-action` if this ever moves off a laptop.

The trade is deliberate: nothing stops you releasing from a dirty tree or
without `.sqlx/`. Those are the checklist above, not code.
