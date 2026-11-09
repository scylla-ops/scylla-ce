# Changelog

All notable changes to `scylla-ce` are in this file: the core and the `scylla-ce` binary.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). `DECISIONS.md` records
the reason for each choice. An entry marked **Breaking** changes what a deploy or an edition must do.

## [Unreleased]

### Changed

- The web app follows the commit that keeps the state of a person on the server: the theme, the
  language, the node style, the last workspace, the seen flags, the tour and the places of the nodes.
- The web app follows the commit that repairs the guided tours: the ring, the card and the hold of a
  dialog.
- **Breaking:** a new backend replaces the old server. An install of the old product needs the
  one-off import: this is not an upgrade in place.
- **Breaking:** the code is in two repositories, in one direction. `scylla-ce` holds the core and is
  public. `scylla-ee` adds to it with plugins, and has no fork of the core.
- **Breaking:** the protos come from the submodule `proto` (`scylla-protos`), and the web app comes
  from the submodule `web` (`scylla-web`). Clone with `--recursive`.
- The code is grouped by subject, ids are opaque, and lists use keyset pages. There is no
  compatibility code: a change deletes the old form.
- **Breaking:** the owners of a workspace hold the built-in role Workspace owner. `Grant` and
  `Revoke` change it, and they also check the new permission `manage_owners`. `AddOwner`, `RemoveOwner`,
  `ListOwners` and `WorkspaceOwner` are gone from the API, and the events `owner.added` and
  `owner.removed` are gone from the log. The rows of `workspace_owners` stay, with no migration.

### Added

**Server**

- `UpdateRole` takes an optional level. A custom role changes its level only while it has no holder,
  else `ROLE_IN_USE`.
- One port for everything: the web app, gRPC, gRPC-Web, the JSON API, the browser routes and the
  webhooks. The binary holds the build of the web app and serves it with no setting.
  `SCYLLA_WEB_DIR` replaces it with a folder. Hashed files have a cache of one year, pages have
  `no-cache`, files go out compressed, and a gRPC call of an unknown service gets `UNIMPLEMENTED`.
- An HTTP/JSON gateway: `POST /api/<package>.<Service>/<Method>`, for the unary methods. An edition
  can add its methods.
- Plugins and an edition builder, with hooks by event and by resource kind.
- Readable call logs with request ids and business events, and JSON lines with no terminal. Each
  refusal goes to the log.
- TLS in the server, a health service, and a stop that ends the running requests.
- The server refuses a public dev key on an address that other machines can reach.

**Notifications**

- An inbox for each person: `NotificationService`, with a count, a list, read, unread, done and mark
  all as read. The routes act on the caller, for a session of a user only.
- A person gets a row for an invitation, a change of a role, an owner or an instance admin, and for
  a change of the account. A row holds a kind and its parameters, so a client shows it in the
  language of the reader. Rows go after `SCYLLA_NOTIFICATION_RETENTION_DAYS` (90), and one person
  keeps at most 1,000.
- `AcceptInvitation` takes the id of the invitation, for a user with its verified email, and
  `DeclineInvitation` tells the person that sent the invitation.
- New mails: a changed password, a closed session after a second use of a refresh token, a locked
  account (at most one for each hour), and a new personal token.
- An edition adds kinds with `EditionBuilder::notification_kind`, and records a fact with
  `ServiceContext::notifier`.

**Access**

- Zero-trust access: Biscuit tokens and SpiceDB permissions, with one permission for each route and
  a typed permission model (`authz!`). An unknown resource gives `PERMISSION_DENIED`.
- Custom roles and the roles of the instance. The members of a workspace and of a project, the
  effective access of a member, and the holders of a role.
- A person needs a role in the workspace to get a role on a project. An invitation keeps this rule.
- `CheckPermissions`, `ListHeldPermissions` and the list of the access to an object.

**Accounts**

- Accounts in Scylla: password sign-in, cookie sessions, the check of an email, the reset of a
  password, HTML mails, the deletion of an account, and handles.
- Sign-in with an identity provider (OIDC or GitHub). There is no automatic link by email: a person
  links an identity with the password of the account at the callback, and gets a mail at each link
  and each unlink.
- A failed sign-in in a browser goes to a page of the web app, with a code.
- Instance admins, and a console of the users.
- Personal tokens, with a switch on the instance and a switch on each workspace.
- `UserStateService`: the state of the web app for the caller, as small JSON values with a version.
  A user has at most 32 keys and 64 KiB, else `USER_STATE_FULL`.

**Workspaces and projects**

- Workspaces with a slug, the role Workspace owner with rows in Postgres, and invitations. Each user can create workspaces up
  to a limit of the instance.
- Hooks of the memberships, which an edition uses to limit the seats.
- The change of a project, and the encrypted secrets of a project.

**Pipelines and agents**

- Pipelines, jobs and their run on the agents, with the logs and their live tail.
- The places of the nodes on the canvas: `SavePipelineLayout`, and a `layout` field in `GetPipeline`,
  `CreatePipeline` and `UpdatePipeline`. A place does not change the version of a pipeline.
- Cron and webhook triggers.
- Apps with join codes and keys, API tokens with limits, CI identities, and the App creator role.
  An App can be disabled and enabled. Keys have names, and record their last use.
- The `scylla-agent` binary, on Linux, macOS and Windows. It runs as a Windows service, joins at its
  first start, and installs in one command (`GET /install.sh`, `GET /install.ps1`).
- The version of the workspace is the version of the release, so the server asks for archives that
  exist. The release workflow stops when the tag is not this version, and checks the version of the
  binary. It adds a build attestation to the archives and signs the image. A tag with a `-` is a
  pre-release, and it does not change `latest`.
- An agent can run the jobs of some projects only. The server counts the jobs of each agent and
  gives its job stats.
- The agent writes a failed attempt in the logs of the node. It names a program that it did not
  find, and tells how to fix a missing shell.
- The agent prints each error in words, with its cause and the path of its file. A refusal for good
  exits with the code 78, and each other error with the code 1.
- `scylla-agent join` checks that it can write its file before it uses the code, so a failed write
  does not lose the code. A file that exists stops the join, and `--force` replaces it.
- `scylla-agent` keeps its files in `~/.scylla-agent` for a user that is not root. `--container-runtime`
  is `auto` by default: the agent looks for `docker`, then `podman`, in the `PATH` at each start, and
  `none` runs no container step. `SCYLLA_URL` alone no longer stops a start.
- The install scripts take `--url`, and they join with a temporary file before any change on the
  machine. They also take `--update`, `--uninstall`, `--binary` and `--force`. The same flags exist
  in `install.ps1`.
- The macOS agent is a LaunchAgent of the user, with no sudo. The Windows service has the account
  `NT SERVICE\ScyllaAgent`, and it finds `sh` of Git for Windows.
- The image has the fixed user id 10001 and the Docker CLI. It runs no container step unless
  `SCYLLA_AGENT_CONTAINER_RUNTIME` is set.
- A container step keeps the settings of the runtime of the agent, and a container empties a work
  folder that the agent cannot delete. The agent trusts the roots of the system too.
- The workflows `agent` and `agent-install` test the agent and its real install on Linux, macOS and
  Windows.
- `GetCapabilities` gives the version of the server and the image of the agent. `SCYLLA_AGENT_IMAGE`
  replaces the image.

**Import**

- A one-off import of the old product: its schema, the users and their passwords, a disabled user,
  and a user with no email.
- `GetAgentStats` counts the jobs of a period: from the day of the creation of the agent to today,
  at most one year. With no period, it counts the last 30 days.
- `ListMembers` and `ListRoleHolders` give `runner` for each actor: true for an App that runs jobs
  (an agent).

### Fixed

- A join code never starts with `-`. A command line read such a code as a flag, so about one code in
  64 failed in `scylla-agent join` and in the install scripts.
- A person who looks at the people of a workspace or of a project no longer sees the instance
  admins, in `ListMembers` and in `GetMemberAccess`. Instance › Users still shows them.
- The rule of the escalation skips the permissions of a machine, and it also applies to an
  invitation.
- A cookie refresh is accepted from the origin of the web app.
- A token is refused when a Datalog query of its claims fails.
- The pretty logs show the random end of the ids.
- Line endings are LF in each file.
- The built-in role Project agent gives its permissions in `ListRoles`: `job_execution.edit` and
  `job_execution.run`. The area `job_execution` is on the project level, so a custom role for apps
  on a project can also hold it.
- The build compiles the protos again when a file of the submodule `proto` changes. Before, a build
  after a move of the submodule could use old generated code.
