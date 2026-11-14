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
- The Rust dependencies are at their latest versions: `croner` 4.0.3, `syn` 3.0.10, `uuid` 1.29.2,
  and the latest patch versions in `Cargo.lock`. A cron schedule with `7L` (the last Sunday of the
  month) now fires.

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
- The API has the contract of the GitHub and GitLab integration. A push, pull request or tag
  trigger needs a linked repository (`NO_REPOSITORY_LINK`), and a file pipeline gets its Git
  triggers from its file (`PIPELINE_READ_ONLY`).
- The permission model of the Git integration: the catalog areas `git_providers` (instance),
  `integrations` (workspace) and `repository` (project), and the cell `jobs.approve` for the runs
  from a fork. A project editor approves a run and links a repository, and a project viewer reads
  the link.
- The agent fetches the code of a job into its work folder before the first node, when the dispatch
  has a checkout. It sends the events of the checkout and its log on the node `.checkout`, and a
  failed checkout fails the job with no node started. The token never goes to `.git/config`, a URL
  or a log. With git 2.31 or newer it is not in the process list. The hello has the feature
  `checkout` only when `git --version` runs.
- A job can have a source: the commit, the branch or the tag, the pull requests and the author in
  the linked repository. `RunPipeline` takes a `ref` (a branch, a tag or a SHA) and gets its commit
  from GitHub or GitLab. Each node gets the `SCYLLA_GIT_*` variables. A job from a fork waits for an
  approval and gets no secret value. `RerunJob` keeps the source and checks the secrets.
  `ListPipelineJobs` and `ListWorkspaceJobs` filter on the branch, the event, the author and the
  pull request.
- The API connects a workspace to GitHub (install of the GitHub App) and to GitLab (the token of a
  service account). It lists, gets and deletes the connections, and lists the repositories of a
  connection. Only the admins of the workspace add, change or delete a connection. No answer and no
  log line has a token.
- `ValidatePipelineFile` reads a pipeline file that a client sends. It gives each error with its
  line and column, and a warning for each secret that the project does not have. `ExportPipeline`
  writes a pipeline as a TOML, YAML or JSON file, with its path in `.scylla/pipelines/`. `GET
  /schemas/pipeline-v1.json` gives the JSON Schema of the pipeline files.
- A job on a commit gets the code of its commit on the agent before the first node. Only an agent
  with the feature `checkout` gets such a job, and a job that waits for one has the `waiting_reason`
  `NO_AGENT_WITH_CHECKOUT`. The dispatch has a token that can only read the repository: an
  installation token of one hour on GitHub, or a GitLab deploy token that the server deletes at the
  end of the job. The job shows the state of its checkout, and `CreatePipeline` and `UpdatePipeline`
  take the checkout settings of a pipeline.
- A project takes its pipelines from the editor or from the pipeline files of its linked repository.
  `SetPipelineSource` reads the files in `.scylla/` at the head of the default branch, archives the
  editor pipelines and writes one pipeline for each file. A file pipeline follows the default
  branch, is read only (`PIPELINE_READ_ONLY`), and gets the triggers of the `on` table of its file.
  A trigger of an archived pipeline does not fire. `ListPipelineFiles` and `GetFilePipeline` give
  the files and their errors. A move back to the editor restores the editor pipelines, and the jobs
  stay.
- `CreateTrigger` and `UpdateTrigger` take the push, pull request and tag triggers with their rules:
  the globs of the branches or of the tags (at most 32 in each list), the events of a pull request,
  the drafts and `[skip ci]`. A project with no linked repository gives `NO_REPOSITORY_LINK`, and a
  file pipeline gives `PIPELINE_READ_ONLY`. `ExportPipeline` writes the enabled Git triggers in the
  `on` table of the file.
- `SetMergeBlock` turns on or off the rule that blocks the merge of a pull request until a pipeline
  passes: one ruleset of Scylla with a required check for each pipeline on GitHub, and "Pipelines
  must succeed" on GitLab. `ListMergeBlocks` gives the state of each rule, with `out_of_sync` when a
  person changed the rule on the provider, and the reason and the page to open when the provider
  refuses.
- Each run on a commit reports its state to GitHub or GitLab as the commit status
  `scylla/<pipeline>`, through an outbox with retries. `VcsReportService` lists the reports of jobs
  and sends a report again. A refused call or an expired GitLab token moves the connection to "Needs
  attention", and the owners and the admins of the workspace get a notification. A warning comes 14
  days before a GitLab token expires.
- `ApproveJob` and `RejectJob` decide on a job from a fork that waits for an approval. Only a person
  with the edit of the project calls them. An approval makes the job pending, the agents get it, and
  it runs with no secret value. A rejection cancels the job with `Rejected by` and the name of the
  person. A job in another state gives `JOB_NOT_AWAITING_APPROVAL`. The report on the commit follows
  the decision.
- The server takes the webhooks of GitHub and GitLab: `POST /integrations/github/webhook`, `POST
  /integrations/github/{app}/webhook` and `POST /integrations/gitlab/{project}/webhook`. It checks
  the signature before any write, keeps each delivery one time, and reads the pushes, the tags, the
  pull requests and the changes of the installations. An uninstall disconnects the connection, a
  suspend stops it, and a repository that leaves the installation stops its link. When the webhooks
  do not arrive or GitLab disables the webhook, the link polls the branches and the open pull
  requests at its interval. A link with webhooks polls each 15 minutes to find a lost delivery.
  `TestDelivery` asks the provider for a webhook, tells if it arrived within 10 seconds, and sets
  the mode of the link.
- On GitHub, each run on a commit now reports its state as the check run `scylla/<pipeline>` by
  default: a title with the nodes that passed, a table of the checkout and the nodes with their
  state, duration and attempts, the errors of an invalid pipeline file, and the last 50 log lines of
  the failed node. A link in the commit status mode still posts commit statuses. "Re-run" of a check
  run or of all the checks on GitHub starts a new run of the job, with the event of a re-run on the
  provider.
- A push, a pull request or a tag of the linked repository starts the editor pipelines whose Git
  triggers match it, as the runner of the triggers. A pipeline has one run for each commit: a second
  event on the commit adds its branch or its pull request to that run. A pull request from a fork
  makes a run that waits for an approval, and the approval tells if the pull request changes
  `.scylla/`. A new commit of the pull request cancels the run of its old commit that waits.
- `VcsAppService` is served. An instance admin makes the GitHub App of a self-hosted instance with
  the GitHub manifest flow (`StartGitHubManifest`, `CompleteGitHubManifest`), and the server seals
  its key and its secrets with the id of its row. `GetVcsApp` gives the App with its webhook URL and
  its last webhook, and gives the App of the settings as managed. `DeleteVcsApp` removes the App,
  and each of its connections becomes disconnected with `APP_UNINSTALLED`.
- A project of files runs its pipeline files at the commit of each push, pull request and tag: each
  file whose `on` table matches the event gets one run for the commit, with the nodes and the
  checkout of the file at that commit. A push to the default branch first syncs the file pipelines.
  A file with errors gives a failed run with no node on each push and pull request, with its errors
  and the report "Invalid pipeline file" (the commit status gives the line of the first error).
  `RunPipeline` of a file pipeline reads its file at the ref, or at the head of the default branch
  with no ref. A schedule or a webhook of a file pipeline, and a Git trigger that a person fires
  now, run at the head of the default branch with the event `trigger`.

### Removed

- A GitHub repository reports only with a check run. The commit status mode of a link and the field
  `report_mode` of `UpdateRepositoryLink` are removed. GitLab continues to report with a commit
  status.

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
- A custom role gets each permission that its pages need, also in another area, and what these need
  in turn. For example, each permission of a project needs the view of the project and of its
  pipelines, the agents need the view of the apps, and the instance access needs the view of the
  users. Thus a page that a role opens does not show a refusal.
- The Windows install script works with no admin rights. In a PowerShell with no admin rights, it
  installs the agent for the user in `%LOCALAPPDATA%`, and a task of the user starts it at each
  sign-in with no window. As administrator, it installs the service `ScyllaAgent`, and
  `-CurrentUser` selects the install for the user. `-Uninstall` removes the two installs.
- A role of the instance with a permission of a workspace or of a project also gets the view of all
  the workspaces, so its holder finds the workspaces where it acts. A custom role that was saved
  before a new dependency gets this dependency at once: the server adds what the permissions of a
  role need each time that it reads the role, checks a route or lists the objects of a role.
- A run on a branch, a tag or a SHA works with a connection of the GitHub App of the instance. It
  gave `APP_MISSING`.
- A new install of the GitHub App on the account of a connection that stopped (an uninstall, a
  suspend or a refused call) takes over this connection in the workspace. The connection keeps its
  id, so its repository links, merge rules and reports work again. Before, the install made a second
  connection, and the links stayed on the old one. A report keeps the latest state of its run while
  the link or its connection is not active, so `RetryJobReport` sends the final state after the
  repair.
- A new GitHub link polls at once when the GitHub App has no active webhook, with the reason
  `APP_HAS_NO_WEBHOOK`, and its connection polls too. `TestDelivery` tells it and moves an existing
  link to the polling mode. Before, a push started a run only at the safety poll after 15 minutes,
  and the test gave a raw error of GitHub.
