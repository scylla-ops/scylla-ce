# Changelog

All notable changes to `scylla-ce` are in this file: the core and the `scylla-ce` binary.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). `DECISIONS.md` records
the reason for each choice. An entry marked **Breaking** changes what a deploy or an edition must do.

## [Unreleased]

### Changed

- **Breaking:** a new backend replaces the old server. An install of the old product needs the
  one-off import: this is not an upgrade in place.
- **Breaking:** the code is in two repositories, in one direction. `scylla-ce` holds the core and is
  public. `scylla-ee` adds to it with plugins, and has no fork of the core.
- **Breaking:** the protos come from the submodule `proto` (`scylla-protos`), and the web app comes
  from the submodule `web` (`scylla-web`). Clone with `--recursive`.
- The code is grouped by subject, ids are opaque, and lists use keyset pages. There is no
  compatibility code: a change deletes the old form.

### Added

**Server**

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

**Workspaces and projects**

- Workspaces with a slug, owners in Postgres, and invitations. Each user can create workspaces up
  to a limit of the instance.
- Hooks of the memberships, which an edition uses to limit the seats.
- The change of a project, and the encrypted secrets of a project.

**Pipelines and agents**

- Pipelines, jobs and their run on the agents, with the logs and their live tail.
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

**Import**

- A one-off import of the old product: its schema, the users and their passwords, a disabled user,
  and a user with no email.

### Fixed

- A person who looks at the people of a workspace or of a project no longer sees the instance
  admins, in `ListMembers` and in `GetMemberAccess`. Instance › Users still shows them.
- The rule of the escalation skips the permissions of a machine, and it also applies to an
  invitation.
- A cookie refresh is accepted from the origin of the web app.
- A token is refused when a Datalog query of its claims fails.
- The pretty logs show the random end of the ids.
- Line endings are LF in each file.
