#!/usr/bin/env bash
# The real path of an agent against a running server: install, connected, a job with its log,
# update, uninstall. It needs curl and jq.
#
#   SERVER=<url> SCYLLA_CE=<server binary> AGENT_BIN=<agent binary> \
#     scripts/agent-install-test.sh <linux|macos|windows|no-service|docker>
#
# PSQL is the psql program, and DATABASE_URL is the database of the server.
set -euo pipefail

mode=$1
token=""
workspace=""

step() {
  echo "::: $*"
}

die() {
  echo "FAILED: $*" >&2
  exit 1
}

api() {
  curl -sS --fail-with-body -H "authorization: Bearer $token" -H 'content-type: application/json' \
    -d "$2" "$SERVER/api/$1"
}

until_true() {
  local what=$1 tries=$2
  shift 2
  for _ in $(seq "$tries"); do
    if "$@"; then
      return 0
    fi
    sleep 2
  done
  die "$what"
}

setup() {
  local user
  user=$("$SCYLLA_CE" user create --email "agent-$RANDOM$RANDOM@example.com" --name agent 2>/dev/null | tail -n 1 | tr -d '\r')
  token=$("$SCYLLA_CE" token create --actor "$user" --ttl 3600 2>/dev/null | tail -n 1 | tr -d '\r')
  workspace=$(api scylla.workspaces.v1.WorkspaceService/ListWorkspaces '{}' | jq -r '.workspaces[0].id.value')
  [ -n "$workspace" ] && [ "$workspace" != null ] || die "no workspace"
}

new_app() {
  local created
  created=$(api scylla.apps.v1.AppService/CreateApp \
    "$(jq -n --arg w "$workspace" --arg n "$1" '{workspaceId: {value: $w}, name: $n, runner: true}')")
  app=$(jq -r '.app.id.value' <<<"$created")
  code=$(jq -r '.joinCode' <<<"$created")
  [ "$code" != null ] || die "no join code"
  api scylla.access.v1.AccessService/Grant \
    "$(jq -n --arg w "$workspace" --arg a "$app" \
      '{object: {kind: "workspace", id: {value: $w}}, role: "runner", actorId: {value: $a}}')" >/dev/null
}

connected() {
  [ "$(api scylla.apps.v1.AppService/GetApp "$(jq -n --arg a "$app" '{appId: {value: $a}}')" | jq -r '.app.connected')" = "${1:-true}" ]
}

not_connected() {
  connected false || connected null
}

# Runs one node and waits for its end. `$1` is the JSON of the node, and `$2` is a text of its log.
run_job() {
  local project pipeline job status
  project=$(api scylla.projects.v1.ProjectService/CreateProject \
    "$(jq -n --arg w "$workspace" --arg n "p$RANDOM$RANDOM" '{workspaceId: {value: $w}, name: $n}')" | jq -r '.project.id.value')
  pipeline=$(api scylla.pipelines.v1.PipelineService/CreatePipeline \
    "$(jq -n --arg p "$project" --argjson node "$1" '{projectId: {value: $p}, name: "install", nodes: [$node]}')" | jq -r '.pipeline.id.value')
  job=$(api scylla.pipelines.v1.PipelineService/RunPipeline \
    "$(jq -n --arg p "$pipeline" '{pipelineId: {value: $p}}')" | jq -r '.job.id.value')
  for _ in $(seq 90); do
    status=$(api scylla.pipelines.v1.JobService/GetJob "$(jq -n --arg j "$job" '{jobId: {value: $j}}')" | jq -r '.job.status')
    case "$status" in
      *COMPLETED*) break ;;
      *FAILED* | *CANCELLED*) die "the job ended with $status" ;;
    esac
    sleep 2
  done
  case "$status" in *COMPLETED*) ;; *) die "the job stays in $status" ;; esac
  "${PSQL:-psql}" "$DATABASE_URL" -tAc "SELECT lines::text FROM job_logs WHERE job_id = '$job'" | grep -q "$2" ||
    die "no text $2 in the log of the job"
}

sh_node() {
  jq -n --arg source "$1" '{id: "hello", script: {source: $source}, timeoutSeconds: 120}'
}

install_sh() {
  curl -fsS "$SERVER/install.sh" | sh -s -- "$@"
}

install_ps1() {
  powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "& ([scriptblock]::Create((irm '$SERVER/install.ps1'))) $*"
}

serve_scripts() {
  curl -fsS "$SERVER/install.sh" | grep -q "SCYLLA_URL='" || die "the server gives no script"
}

expect_failure() {
  local what=$1
  shift
  if "$@" >/dev/null 2>&1; then
    die "$what"
  fi
}

test_linux() {
  local unit=/etc/systemd/system/scylla-agent.service
  new_app linux

  step "a bad code changes nothing"
  expect_failure "a bad code passed" install_sh wrong-code --url "$SERVER" --binary "$AGENT_BIN"
  [ ! -e "$unit" ] && [ ! -e /usr/local/bin/scylla-agent ] && ! id scylla-agent >/dev/null 2>&1 ||
    die "a bad code changed the machine"

  step "install"
  install_sh "$code" --url "$SERVER" --binary "$AGENT_BIN"
  until_true "the agent did not connect" 30 connected
  systemctl is-active --quiet scylla-agent || die "the service does not run"
  [ "$(sudo stat -c '%U %a' /var/lib/scylla-agent/agent.json)" = "scylla-agent 600" ] || die "the mode of the file"

  step "a job, on the host"
  run_job "$(sh_node 'echo hello-from-the-agent; id -un')" hello-from-the-agent

  step "a second install stops before any change"
  expect_failure "a second install passed" install_sh "$code" --url "$SERVER" --binary "$AGENT_BIN"
  systemctl is-active --quiet scylla-agent || die "a second install stopped the service"

  step "update"
  install_sh --update --binary "$AGENT_BIN"
  until_true "the agent did not come back" 30 connected
  run_job "$(sh_node 'echo after-the-update')" after-the-update

  step "a refused key stops the service"
  local key
  key=$(api scylla.apps.v1.AppService/GetApp "$(jq -n --arg a "$app" '{appId: {value: $a}}')" | jq -r '.keys[0].id.value')
  api scylla.apps.v1.AppService/RevokeAppKey "$(jq -n --arg a "$app" --arg k "$key" '{appId: {value: $a}, keyId: {value: $k}}')" >/dev/null
  sudo systemctl restart scylla-agent || true
  sleep 10
  [ "$(systemctl show -p Result --value scylla-agent)" = exit-code ] || die "the result of the service"
  [ "$(systemctl show -p ExecMainStatus --value scylla-agent)" = 78 ] || die "the exit code of the service"
  [ "$(systemctl show -p ActiveState --value scylla-agent)" = failed ] || die "the service starts again"

  step "uninstall"
  install_sh --uninstall
  [ ! -e "$unit" ] && [ ! -e /usr/local/bin/scylla-agent ] && [ ! -e /var/lib/scylla-agent ] || die "files stay"
  ! id scylla-agent >/dev/null 2>&1 || die "the user stays"
}

test_macos() {
  local plist="$HOME/Library/LaunchAgents/com.scylla.agent.plist" uid
  uid=$(id -u)
  new_app macos

  step "a bad code changes nothing"
  expect_failure "a bad code passed" install_sh wrong-code --url "$SERVER" --binary "$AGENT_BIN"
  [ ! -e "$plist" ] && [ ! -e "$HOME/.scylla-agent" ] || die "a bad code changed the machine"

  step "install"
  install_sh "$code" --url "$SERVER" --binary "$AGENT_BIN"
  until_true "the agent did not connect" 30 connected
  launchctl print "gui/$uid/com.scylla.agent" >/dev/null || die "launchd does not have the agent"

  step "a job"
  run_job "$(sh_node 'echo hello-from-the-agent')" hello-from-the-agent

  step "a second install stops before any change"
  expect_failure "a second install passed" install_sh "$code" --url "$SERVER" --binary "$AGENT_BIN"
  launchctl print "gui/$uid/com.scylla.agent" >/dev/null || die "a second install stopped the agent"

  step "update"
  install_sh --update --binary "$AGENT_BIN"
  until_true "the agent did not come back" 45 connected

  step "uninstall"
  install_sh --uninstall
  [ ! -e "$plist" ] && [ ! -e "$HOME/.scylla-agent" ] || die "files stay"
  ! launchctl print "gui/$uid/com.scylla.agent" >/dev/null 2>&1 || die "launchd keeps the agent"
  until_true "the server still sees the agent" 30 not_connected
}

service_gone() {
  ! sc.exe query ScyllaAgent >/dev/null 2>&1
}

test_windows() {
  local exe='C:\Program Files\Scylla Agent\scylla-agent.exe' data='C:\ProgramData\scylla-agent'
  new_app windows

  step "a bad code changes nothing"
  expect_failure "a bad code passed" install_ps1 -JoinCode wrong-code -Url "$SERVER" -Binary "$(cygpath -w "$AGENT_BIN")"
  [ ! -e "$exe" ] && [ ! -e "$data" ] || die "a bad code changed the machine"

  step "install"
  install_ps1 -JoinCode "$code" -Url "$SERVER" -Binary "$(cygpath -w "$AGENT_BIN")"
  until_true "the agent did not connect" 30 connected
  sc.exe qc ScyllaAgent | grep -qi 'NT SERVICE\\ScyllaAgent' || die "the account of the service"

  step "a script job, with sh of Git"
  run_job "$(sh_node 'echo hello-from-the-agent; uname -s')" hello-from-the-agent

  step "a second install stops before any change"
  expect_failure "a second install passed" install_ps1 -JoinCode "$code" -Url "$SERVER" -Binary "$(cygpath -w "$AGENT_BIN")"
  sc.exe query ScyllaAgent | grep -q RUNNING || die "a second install stopped the service"

  step "update"
  install_ps1 -Update -Binary "$(cygpath -w "$AGENT_BIN")"
  until_true "the agent did not come back" 30 connected

  step "uninstall"
  install_ps1 -Uninstall
  [ ! -e "$exe" ] && [ ! -e "$data" ] || die "files stay"
  until_true "the service stays" 15 service_gone
}

test_no_service() {
  local home
  home=$(mktemp -d)
  new_app no-service

  step "install with no service"
  HOME="$home" install_sh "$code" --url "$SERVER" --binary "$AGENT_BIN" --no-service | tee "$home/output"
  [ -x "$home/.scylla-agent/scylla-agent" ] || die "no binary"
  [ "$(stat -f %Lp "$home/.scylla-agent/agent.json" 2>/dev/null || stat -c %a "$home/.scylla-agent/agent.json")" = 600 ] || die "the mode of the file"

  step "the printed command runs the agent"
  local command
  command=$(sed -n 's/^start the agent with: //p' "$home/output")
  # shellcheck disable=SC2086
  HOME="$home" $command &
  local agent=$!
  trap 'kill "$agent" 2>/dev/null || true' EXIT
  until_true "the agent did not connect" 30 connected

  step "a job"
  run_job "$(sh_node 'echo hello-from-the-agent')" hello-from-the-agent

  step "a second install stops"
  HOME="$home" expect_failure "a second install passed" install_sh "$code" --url "$SERVER" --binary "$AGENT_BIN" --no-service

  step "update and uninstall"
  HOME="$home" install_sh --update --no-service --binary "$AGENT_BIN"
  kill "$agent"
  wait "$agent" 2>/dev/null || true
  trap - EXIT
  HOME="$home" install_sh --uninstall --no-service
  [ ! -e "$home/.scylla-agent" ] || die "files stay"
  rm -rf "$home"
}

app_runtime() {
  api scylla.apps.v1.AppService/GetApp "$(jq -n --arg a "$app" '{appId: {value: $a}}')" | jq -r '.app.host.containerRuntime // ""'
}

# AGENT_IMAGE is the image of the agent. The server is on the loopback, so the agent uses the host network.
test_docker() {
  new_app docker-shell
  step "the shell agent"
  docker run -d --name scylla-agent --restart unless-stopped --network host -e SCYLLA_URL="$SERVER" -e SCYLLA_JOIN_CODE="$code" \
    -v scylla-agent:/var/lib/scylla-agent "$AGENT_IMAGE" >/dev/null
  until_true "the agent did not connect" 30 connected
  [ -z "$(app_runtime)" ] || die "the shell agent has a runtime"
  run_job "$(sh_node 'echo hello-from-the-image; id -u; git --version; bash --version')" hello-from-the-image

  step "a restart and an update use no code"
  docker restart scylla-agent >/dev/null
  until_true "the agent did not come back" 30 connected
  docker rm -f scylla-agent >/dev/null
  docker run -d --name scylla-agent --restart unless-stopped --network host -v scylla-agent:/var/lib/scylla-agent "$AGENT_IMAGE" >/dev/null
  until_true "the agent did not come back" 30 connected
  docker rm -f scylla-agent >/dev/null
  docker volume rm scylla-agent >/dev/null
  until_true "the server still sees the agent" 30 not_connected

  new_app docker-socket
  step "the agent with container steps"
  sudo install -d -o 10001 -g 10001 -m 700 /var/lib/scylla-agent
  docker run -d --name scylla-agent --restart unless-stopped --network host --group-add "$(stat -c %g /var/run/docker.sock)" \
    -e SCYLLA_URL="$SERVER" -e SCYLLA_JOIN_CODE="$code" -e SCYLLA_AGENT_CONTAINER_RUNTIME=docker \
    -v /var/run/docker.sock:/var/run/docker.sock -v /var/lib/scylla-agent:/var/lib/scylla-agent "$AGENT_IMAGE" >/dev/null
  until_true "the agent did not connect" 30 connected
  [ "$(app_runtime)" = docker ] || die "the agent has no runtime"
  run_job "$(jq -n '{id: "step", timeoutSeconds: 180, container: {image: "alpine:3.22"},
    script: {source: "mkdir sub; echo from-a-container > sub/file.txt; cat sub/file.txt; id -u"}}')" from-a-container

  step "the work folder goes, with a file of root in it"
  work_is_empty() {
    [ -z "$(sudo ls -A /var/lib/scylla-agent/work 2>/dev/null)" ]
  }
  until_true "the work folder stays" 15 work_is_empty
  docker rm -f scylla-agent >/dev/null
  sudo rm -rf /var/lib/scylla-agent
}

setup
serve_scripts
"test_${mode//-/_}"
echo "PASSED: $mode"
