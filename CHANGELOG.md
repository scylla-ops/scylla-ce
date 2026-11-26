# Changelog

All notable changes to `scylla-ce` are in this file: the core and the `scylla-ce` binary.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). `DECISIONS.md` records
the reason for each choice. An entry marked **Breaking** changes what a deploy or an edition must do.

## [Unreleased]

### Added

- Custom project icons. A project keeps a glyph with a color token or a `#RRGGBB` color, or an
  uploaded image. The new migration only adds nullable columns. The server re-encodes each uploaded
  image (PNG, JPEG or WebP, 1 MB max) as a square PNG of 256 px, and the public route `GET
  /images/{id}` serves it with `nosniff` and a sandbox CSP. The server refuses an SVG. Only a member
  that can update the project changes its icon. The `image` crate is a new dependency.
- `ListProjects` searches the projects of a workspace in their name, description, repository and
  pipeline names, sorts them (position, name, last job, failed first), keeps only the failing or the
  running ones, and returns their counts. `ListRepositoryLinks` returns the repository links of a
  workspace, or of at most 100 projects, in one call, with no link for a project that the caller
  cannot read. No migration.
- A photo for each user and an icon for each App. A user sets an image or the color of the initials
  with `UpdateProfile`. An admin of the workspace sets the icon of an App with `SetAppIcon`: a glyph
  with a color, an image, or the default icon. The images go through the image service of the
  project icons. The server deletes the old image when it gets a new one, when a user deletes the
  account, and when an admin deletes the App. `ListMembers` and the export of an account give the
  photo. The new migration only adds nullable columns.
- Groups of projects in a workspace. The new migration adds the table `project_groups` and the
  nullable columns `projects.group_id` and `projects.position`. The owners, the admins and the
  members of a workspace create, rename, reorder and delete its groups, and the editors of a project
  also move it to a group. When a group is deleted, its projects go to no group. `ListProjects`
  finds the projects of one group, the projects with no group, or at most 100 given projects, and
  the position order uses the place of each project in its group.

### Fixed

- Account mails with fewer spam signals, and a reference for each sent mail. The HTML body has no
  hidden preheader and no filler of invisible characters. The SMTP mailer gives the host of
  `SCYLLA_APP_URL` in `EHLO`, not `[127.0.0.1]` (a name in the path of `SCYLLA_SMTP_URL` wins). The
  log line `mail.sent` has the `Message-ID` of the mail and the answer of the mail server with its
  queue id. For the editions, `Mailer::send_traced` (with a default body) returns this
  `MailReceipt`, and `SmtpMailer::new_for_app` gives the host of the web app in `EHLO`.

## [0.7.2] - 2026-11-25

### Added

- Scylla pipelines next to file pipelines. In a project that takes its pipelines from the repository
  files, the Scylla pipelines stay, and `CreatePipeline` makes new ones. A file of the default
  branch that takes the key of a Scylla pipeline wins: the Scylla pipeline sends no report, and a
  new name with the key of a file gives `PIPELINE_KEY_TAKEN`. The pipeline summaries return both
  kinds with their source. The new migration only adds nullable columns.
- Git events for a Scylla pipeline in such a project. `SetPipelineGitEvents` lets a workspace admin
  allow or revoke its push, pull request and tag triggers (permission `set_pipeline_git_events`, in
  the new area `git_events`). Only the admins of a workspace hold it: no role of a project has it.
- `ResendVerificationEmail`. A person with an email that is not verified asks for a new verify
  link. The server allows 5 new links an hour for one email and 20 for one IP address, and gives
  the same answer for each email. `ListSignInMethods` gives the sender of the mails
  (`SCYLLA_MAIL_FROM`).
- The mail outbox. A change puts its mail in the outbox in its transaction, and one task of the
  server sends it. After a transient error, the server tries again for 24 hours. A sent mail keeps
  no content, and each mail has a `Message-ID` on the domain of the sender. The log shows
  `mail.sent` with the number of attempts. The new migration only adds a table.

### Changed

- **Breaking** for an edition with its own mailer: `MailError` has the fields `reason` and
  `permanent`, made with `MailError::transient` or `MailError::permanent`, and `Mailer::delivers`
  tells if a mailer sends the mails. In the testkit, `Mailbox::to` and `Mailbox::token` are async:
  they first wait for the first send of each mail of the outbox. `fail_next` and `refuse_sends` give
  transient and permanent errors.
- A sign-up with an email that is not verified sends a new verify link to the pending account, at
  most one each 60 seconds, and the account does not change.

### Fixed

- The creation of a pipeline from the create page saves the agent labels with the pipeline, so its
  first job goes only to an agent with each label. A test of the API covers `CreatePipeline` with
  labels.

## [0.7.1] - 2026-11-24

### Added

- Agent labels. An agent gets automatic labels from its host (`linux`, `macos` or `windows`,
  `x64` or `arm64`, and `container` when it has a container runtime), and the admins of a
  workspace give it more with `SetAppLabels` (permission `set_app_labels`). A pipeline file
  asks labels with `agent.labels` (16 at most). A job keeps the labels of its pipeline, a
  rerun keeps them, and an agent claims only the jobs whose labels it has. A job that waits
  for such an agent shows it on its page and in the queued GitHub check run. The new migration
  only adds.

### Changed

- `scylla-core/migrations.lock` holds the checksum of each migration, not only of the released ones:
  a new migration adds its line in the change that adds it, and a test fails when a migration has no
  line or a line changed. `scripts/check-structure.sh` also fails when a migration of the last
  release tag changes or goes. Each upgrade from 0.7.0 keeps the data, with no reset of the
  database.
- The checks of a pull request run in Scylla, on a Scylla agent (`.scylla/pipelines/ci.yaml`), not
  in GitHub Actions. The jobs of the agent for macOS and Windows and the release stay in GitHub
  Actions for now.

## [0.7.0] - 2026-11-24

A new backend: a new server, a new API, a new agent and a new web app. This is not an upgrade in
place: read the **Breaking** entries of Changed before you move from 0.6.

### Added

**Server**

- One port for everything: the web app, gRPC, gRPC-Web, an HTTP/JSON gateway for the unary methods
  (`POST /api/<package>.<Service>/<Method>`), the browser routes and the webhooks. The binary holds
  the build of the web app and serves it with no setting, and `SCYLLA_WEB_DIR` replaces it with a
  folder. Hashed files have a cache of one year, pages have `no-cache`, and files go out compressed.
- The answers of the web app have a `Content-Security-Policy` that allows only the own origin, the
  API and the avatar hosts of GitHub, GitLab and Gravatar, and `Referrer-Policy:
  strict-origin-when-cross-origin`. With HTTPS, the cookies of the server have the prefix `__Host-`.
- TLS in the server, a health service, the gRPC reflection v1 and v1alpha (a client such as
  `grpcurl` lists the services and calls them with no copy of the protos), and a stop that ends the
  running requests. The server refuses a public dev key on an address that other machines can
  reach.
- Readable call logs with request ids and business events, and JSON lines with no terminal. Each
  refusal goes to the log. `SCYLLA_LOG_CLIENT_IP` keeps the full, the truncated or no IP address of
  a client.
- A build that is not a release shows its commit in its version (`0.7.0+240da00`, or `0.7.0+dev`):
  in `--version` of the server and of the agent, in the hello of the agent and in
  `GetCapabilities`. Only the release workflow (`SCYLLA_RELEASE=true`) gives the bare version.
  `GetCapabilities` also gives the agent protocols of the server and the image of the agent.
- **Edition contract:** plugins and an edition builder, with hooks by event and by resource kind
  (`BeforeDelete` too, and `CreateContext::with_object`), the hooks of the memberships, background
  tasks (`EditionBuilder::task`), notification kinds (`EditionBuilder::notification_kind`,
  `ServiceContext::notifier`), the export and the deletion of the data of a user (`ExportUser`,
  `AfterDeleteUser`), and the methods of an edition on the JSON gateway.

**Access and accounts**

- Zero-trust access: Biscuit tokens and SpiceDB permissions, with one permission for each route and
  a typed permission model (`authz!`). An unknown resource gives `PERMISSION_DENIED`, the same
  answer as a refusal. `CheckPermissions`, `ListHeldPermissions` and the list of the access to an
  object.
- Custom roles, built-in roles and roles of the instance, with a catalog of the permissions. A
  permission brings each permission that its pages need, also in another area. The owners of a
  workspace hold the built-in role Workspace owner: only an owner or an instance admin gives it
  (`manage_owners`), and the last owner stays. A person needs a role in the workspace to get a role
  on a project, and an invitation keeps this rule. The members of a workspace and of a project, the
  effective access of a member and the holders of a role show no instance admin.
- Accounts: a password sign-in, cookie sessions, the check of an email, the reset of a password,
  handles and the deletion of an account. A sign-in with an identity provider (OIDC or GitHub)
  never links an account by its email: a person links an identity with the password of the
  account, and gets a mail at each link and each unlink. A failed sign-in in a browser goes to a
  page of the web app, with a code.
- The mails of the accounts have the look of the web app: the logo, a badge, the facts of the
  event, one button and a dark theme. Mails tell a changed password, a session closed after a
  second use of its refresh token, a locked account and a new personal token.
- Instance admins and a console of the users. Personal tokens, with a switch on the instance and on
  each workspace. `UserStateService` keeps the state of the web app of each user, at most 32 keys
  and 64 KiB.

**Workspaces, notifications and activity**

- Workspaces with a slug, an IANA time zone (`UTC` for a new workspace) and invitations, and a
  limit of the workspaces of each user. The change of a project, and the encrypted secrets of a
  project.
- An inbox for each person (`NotificationService`): an invitation, a change of a role, of an owner
  or of an instance admin, and a change of the account. A row holds a kind and its parameters, so a
  client shows it in the language of the reader. Rows go after
  `SCYLLA_NOTIFICATION_RETENTION_DAYS` (90), and one person keeps at most 1,000.
  `AcceptInvitation` and `DeclineInvitation` answer an invitation from its row.
- The activity feed of a workspace (`ListActivity`): the end of each job, and each change of the
  triggers, the secrets, the members, the Git connections and the agents. A row shows only to the
  readers of its area, and goes after 30 days.

**Pipelines and jobs**

- Pipelines of 1 to 256 nodes, jobs and their run on the agents, with the logs and their live tail.
  The places of the nodes on the canvas (`SavePipelineLayout`) do not change the version of a
  pipeline.
- Conditional nodes. A switch checks its cases (texts, numbers, variables and the outputs of earlier
  nodes) and opens one port or more, with no process. A merge joins branches. The port `error` of a
  run node takes its failure, and the job still fails. A step writes `KEY=value` lines to
  `$SCYLLA_OUTPUT`, and a node after a failure gets `SCYLLA_ERROR`. A node on a path that the job
  did not take is skipped. `ExportPipeline` refuses a pipeline with conditional nodes
  (`CONDITIONAL_NODES`), because a pipeline file has no switch, no merge and no need with a port.
- The library of a workspace (`FragmentService`): fragments are groups of nodes that the pipelines
  of the workspace share. A fragment has versions that never change, and the favorites of each
  reader. A pipeline keeps the nodes of a fragment and a link to its version
  (`Pipeline.fragment_links`): a new version is only proposed. A delete keeps the pipelines that
  link the fragment. A fragment can come from a package of an outside registry (`FragmentOrigin`):
  a linked fragment takes the versions of the package with `AcceptOriginVersion`, and a fork
  changes its own. New catalog cells `library.view`, `library.create` and `library.manage`.
- Cron and webhook triggers. A cron schedule uses the time zone of the workspace, and a change of
  the zone moves the next fires. A webhook can have conditions on a header or on a value of the
  body (is, matches or is one of): a delivery that does not match starts no job and shows as
  skipped. The history keeps each fire and each missed cron time for 30 days
  (`ListWorkspaceTriggers`, `ListTriggerFires`, `GetTriggerFireStats`).
- The data of the dashboard: `ListPipelineSummaries` (the recent jobs, the health and the triggers
  of the pipelines), `GetJobStats` (the counts, the durations and the waits of the jobs of a period,
  in buckets of the time zone of the workspace), `ListAttentionItems` (the open items that need a
  person) and `DashboardService` (the saved layouts and the team default). Each job keeps its
  origin, and the job lists filter on the states, the origins, the time, the projects and the
  pipelines.

**Apps and agents**

- Apps with join codes and named keys, API tokens with limits, CI identities, and the role App
  creator. An app can be disabled and enabled, and a key records its last use.
- The `scylla-agent` binary, on Linux, macOS and Windows, installs in one command
  (`GET /install.sh`, `GET /install.ps1`). The script downloads the agent of the release of the
  server, checks its SHA-256, and joins with a temporary file before any change on the machine. It
  takes `--url`, `--capacity`, `--container-runtime`, `--no-service`, `--binary`, `--update`,
  `--uninstall` and `--force`. The agent is a systemd service on Linux, a LaunchAgent of the user on
  macOS, and the service `ScyllaAgent` on Windows, or a task of the user with no admin rights.
  `SCYLLA_AGENT_DOWNLOADS_URL` and `SCYLLA_AGENT_IMAGE` replace the downloads and the image for a
  mirror.
- The image `ghcr.io/scylla-ops/scylla-agent`, with the user id 10001 and the Docker CLI. At each
  tag, the release workflow builds the archives of six platforms, `SHA256SUMS` with a build
  attestation, and a signed image.
- The agent runs process steps and container steps (`--container-runtime auto` looks for `docker`,
  then `podman`). It writes a failed attempt in the logs of the node, prints each error with its
  cause, and exits with the code 78 on a refusal for good. It sends the CPU use and the used memory
  of its machine each minute, and the server keeps the samples for 7 days (`GetAgentMetrics`,
  `GetSlotUsage`).
- An agent tells its agent protocol and its build commit in its hello. The server compares the
  protocol, not the version text, gives each agent a compatibility and its missing features, and
  refuses an agent under its minimum protocol (`AGENT_TOO_OLD`). An agent can run the jobs of some
  projects only, and the server gives the job stats of each agent for a period.

**GitHub and GitLab**

- A workspace connects to GitHub (the Scylla GitHub App) and to GitLab (the token of a service
  account). An instance admin makes the GitHub App of a self-hosted server with the GitHub manifest
  flow (`VcsAppService`). No answer and no log line has a token.
- A project links a repository, and takes its pipelines from the editor or from the pipeline files
  of the repository (`.scylla/pipelines/`, in TOML, YAML or JSON). A file pipeline is read only, and
  gets the triggers of the `on` table of its file. `ValidatePipelineFile` gives each error with its
  line and column, `ExportPipeline` writes a pipeline as a file, and `GET /schemas/pipeline-v1.json`
  gives the JSON Schema of the files.
- Push, pull request and tag triggers, with the globs of the branches and of the tags, the events
  and the drafts of a pull request, and `[skip ci]`. The server takes the webhooks of GitHub and
  GitLab and checks their signature before any write, and a link polls when the webhooks do not
  arrive. A pipeline has one run for each commit.
- A job on a commit gets its code on the agent before its first node, with a token that can only
  read the repository, and each node gets the `SCYLLA_GIT_*` variables. A run from a fork waits for
  an approval and gets no secret value (`ApproveJob`, `RejectJob`).
- Each run reports its state to the provider, through an outbox with retries: a check run on GitHub
  and a commit status on GitLab. `SetMergeBlock` blocks the merge of a pull request until a pipeline
  passes. A refused call or an expiring GitLab token moves the connection to "Needs attention", and
  the owners and the admins of the workspace get a notification.

**Legal and privacy**

- The legal settings of the instance (`LegalService`): the operator, the privacy contact and seven
  legal documents, each a link or a text in Markdown, with what changes in each version. The footer
  of each mail links to them.
- The acceptance of the terms of service. A sign-up with a password accepts their current version,
  and `AcceptTerms` keeps the acceptance of each new version, with its method as proof. The proof of
  a deleted account stays for `SCYLLA_TERMS_PROOF_RETENTION_DAYS` (1827, 5 years) days, with a hash
  of its email.
- `ExportAccountData` gives the data of an account as one JSON document, with no secret.
  `ConsentService.RecordConsent` keeps each choice about cookies.

**Import**

- `scylla-ce import` imports a 0.6 database one time, into a new database with no user. It copies
  the users and their passwords, the instance admins, the organizations as workspaces with their
  owners, the roles, the projects, the secrets (sealed again with `SCYLLA_OLD_SECRET_KEY`) and the
  pipelines. It prints what it imported and each row that it left, with the reason.

### Changed

- **Breaking:** a new backend replaces the 0.6 server. This is not an upgrade in place: install 0.7
  with a new database, then run `scylla-ce import` one time. The users sign in with their old
  passwords, and their sessions do not come over. The apps, the agents, the jobs, the logs and the
  triggers do not come over: each agent joins again with a new join code.
- **Breaking:** the server needs SpiceDB with a Postgres datastore, and new settings:
  `SCYLLA_SPICEDB_ENDPOINT`, `SCYLLA_SPICEDB_KEY`, `SCYLLA_TOKEN_PRIVATE_KEY` (`token keygen`),
  `SCYLLA_SECRET_KEY` (`secret keygen`), `SCYLLA_API_URL`, `SCYLLA_APP_URL`, `SCYLLA_SMTP_URL`,
  `SCYLLA_MAIL_FROM` and `SCYLLA_SIGN_UP_MODE` (`invite_only` by default). Make new keys: a server
  that other machines can reach does not start with a public dev key of the repository.
- **Breaking:** run `scylla-ce migrate` as a job before `scylla-ce serve`. It applies the migrations
  and writes the SpiceDB schema, and a server stops at its start when a migration is missing.
- **Breaking:** the code is in two repositories, in one direction. `scylla-ce` holds the core and is
  public, and `scylla-ee` adds to it with plugins. The protos come from the submodule `proto`
  (`scylla-protos`), and the web app from the submodule `web` (`scylla-web`): clone with
  `--recursive`.
- The schema only grows from 0.7.0: a schema change is a new migration, and a released migration
  never changes, so an upgrade keeps the data. `scylla-core/migrations.lock` holds the checksum of
  each released migration, and a test fails when a released migration changes or goes, or when a
  migration has a CR. The SpiceDB schema also only grows.

[Unreleased]: https://github.com/scylla-ops/scylla-ce/compare/v0.7.2...HEAD
[0.7.2]: https://github.com/scylla-ops/scylla-ce/compare/v0.7.1...v0.7.2
[0.7.1]: https://github.com/scylla-ops/scylla-ce/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/scylla-ops/scylla-ce/compare/v0.6.0...v0.7.0
