#!/bin/sh
# Installs the Scylla agent on Linux or macOS, joins its App, and starts it as a service.
#
#   curl -fsSL <server>/install.sh | sh -s -- <join-code> --url <server> [options]
#   curl -fsSL <server>/install.sh | sh -s -- --update
#   curl -fsSL <server>/install.sh | sh -s -- --uninstall
#
# Options:
#   --url <url>                 the address of the server for this machine
#   --capacity <n>              the largest number of jobs at the same time (default 1)
#   --container-runtime <name>  auto, docker, podman, or none (default auto)
#   --no-service                no sudo and no service: all files in ~/.scylla-agent
#   --binary <file>             use this binary and download nothing
#   --force                     replace the file of a machine that joined before, with a new key
#
# SCYLLA_AGENT_DOWNLOADS_URL replaces the folder of the downloads.
set -eu

SCYLLA_URL='@SCYLLA_URL@'
VERSION='@SCYLLA_VERSION@'
DEFAULT_DOWNLOADS='@DOWNLOADS_URL@'
DOWNLOADS="${SCYLLA_AGENT_DOWNLOADS_URL:-$DEFAULT_DOWNLOADS}"
SYSTEM_USER=scylla-agent
LABEL=com.scylla.agent
UNIT=/etc/systemd/system/scylla-agent.service

fail() {
  echo "scylla-agent install: $*" >&2
  exit 1
}

parse() {
  action=install
  code=""
  url=""
  binary=""
  capacity=1
  runtime=auto
  service=1
  force=0
  while [ $# -gt 0 ]; do
    case "$1" in
      --url | --binary | --capacity | --container-runtime)
        [ $# -ge 2 ] || fail "$1 needs a value"
        case "$1" in
          --url) url=$2 ;;
          --binary) binary=$2 ;;
          --capacity) capacity=$2 ;;
          *) runtime=$2 ;;
        esac
        shift 2
        ;;
      --no-service)
        service=0
        shift
        ;;
      --force)
        force=1
        shift
        ;;
      --update | --uninstall)
        action=${1#--}
        shift
        ;;
      -*) fail "unknown option $1" ;;
      *)
        [ -z "$code" ] || fail "give one join code only"
        code=$1
        shift
        ;;
    esac
  done
  case "$capacity" in '' | *[!0-9]* | 0) fail "--capacity needs a number from 1" ;; esac
  case "$runtime" in '' | *[!a-z0-9_-]*) fail "--container-runtime needs a name, for example docker" ;; esac
  if [ "$action" = install ]; then
    [ -n "$code" ] || fail "give the join code of the app: sh -s -- <join-code> --url <server>"
    url=${url:-$SCYLLA_URL}
    case "$url" in http://* | https://*) ;; *) fail "--url needs an address with http:// or https://" ;; esac
  else
    [ -z "$code" ] || fail "--$action needs no join code"
  fi
}

find_target() {
  os=$(uname -s)
  arch=$(uname -m)
  case "$arch" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) fail "no build for the CPU $arch" ;;
  esac
  case "$os" in
    Linux) target="$arch-unknown-linux-musl" ;;
    Darwin) target="$arch-apple-darwin" ;;
    *) fail "no build for $os; on Windows, use install.ps1" ;;
  esac
}

# On Linux, --update and --uninstall find an agent that has no service in ~/.scylla-agent.
find_folders() {
  SUDO=""
  user=""
  if [ "$os" = Linux ] && [ "$action" != install ] && [ ! -e "$UNIT" ] &&
    [ ! -e /usr/local/bin/scylla-agent ] && [ -e "${HOME:-}/.scylla-agent/scylla-agent" ]; then
    service=0
  fi
  if [ "$service" = 0 ] || [ "$os" = Darwin ]; then
    data="${HOME:?set HOME to your home folder}/.scylla-agent"
    bin="$data/scylla-agent"
  else
    data=/var/lib/scylla-agent
    bin=/usr/local/bin/scylla-agent
    user=$SYSTEM_USER
    if [ "$(id -u)" -ne 0 ]; then
      command -v sudo >/dev/null 2>&1 || fail "run as root or install sudo, or add --no-service"
      SUDO=sudo
    fi
  fi
  config="$data/agent.json"
  domain="gui/$(id -u)"
  plist="${HOME:-}/Library/LaunchAgents/$LABEL.plist"
  flags="--capacity $capacity"
  if [ "$runtime" != auto ]; then
    flags="$flags --container-runtime $runtime"
  fi
}

check_system() {
  if [ "$os" = Darwin ] && [ "$service" = 1 ]; then
    [ "$(id -u)" -ne 0 ] || fail "run this as your user, not as root: the agent is a LaunchAgent of your user"
    launchctl print "$domain" >/dev/null 2>&1 ||
      fail "launchd has no session for this user. Sign in on the Mac and run this in Terminal, or add --no-service"
  fi
  if [ -n "$user" ] && [ "$action" = install ] && ! id "$user" >/dev/null 2>&1; then
    command -v useradd >/dev/null 2>&1 || fail "no useradd on this machine. Make the user $user, or add --no-service"
  fi
}

download() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$1" -o "$2"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$2" "$1"
  else
    fail "install curl or wget"
  fi
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

get_binary() {
  if [ -n "$binary" ]; then
    [ -f "$binary" ] || fail "$binary does not exist"
    cp "$binary" "$tmp/scylla-agent"
    chmod 755 "$tmp/scylla-agent"
  else
    archive="scylla-agent-$target.tar.gz"
    echo "downloading scylla-agent $VERSION for $target"
    download "$DOWNLOADS/$archive" "$tmp/$archive" || fail "the download of $DOWNLOADS/$archive failed"
    download "$DOWNLOADS/SHA256SUMS" "$tmp/SHA256SUMS" || fail "the download of the checksums failed"
    expected=$(awk -v file="$archive" '$2 == file || $2 == "*" file { print $1 }' "$tmp/SHA256SUMS")
    [ -n "$expected" ] || fail "no checksum for $archive"
    [ "$expected" = "$(sha256 "$tmp/$archive")" ] || fail "the checksum of $archive does not match"
    tar -xzf "$tmp/$archive" -C "$tmp"
  fi
  found=$("$tmp/scylla-agent" --version 2>/dev/null) ||
    fail "the binary does not run from $tmp. It is for another system, or TMPDIR must be a folder that runs programs"
  if [ "$found" != "scylla-agent $VERSION" ]; then
    echo "warning: this is $found, and the server has the version $VERSION" >&2
  fi
}

stop_service() {
  if [ -n "$user" ] && [ -e "$UNIT" ]; then
    $SUDO systemctl stop scylla-agent >/dev/null 2>&1 || true
  elif [ "$os" = Darwin ] && [ -e "$plist" ]; then
    launchctl bootout "$domain/$LABEL" >/dev/null 2>&1 || true
  fi
}

make_user() {
  if ! id "$user" >/dev/null 2>&1; then
    $SUDO useradd --system --home-dir "$data" --shell /usr/sbin/nologin "$user"
  fi
  if [ "$runtime" != none ] && getent group docker >/dev/null 2>&1; then
    $SUDO usermod -aG docker "$user"
    echo "the user $user is in the group docker, which gives root rights on this machine"
  fi
}

put_files() {
  if [ -n "$user" ]; then
    $SUDO install -d -m 700 -o "$user" "$data" "$data/work"
    $SUDO install -m 755 "$tmp/scylla-agent" "$bin"
    $SUDO install -m 600 -o "$user" "$tmp/agent.json" "$config"
  else
    mkdir -p "$data/work"
    chmod 700 "$data"
    install -m 755 "$tmp/scylla-agent" "$bin"
    install -m 600 "$tmp/agent.json" "$config"
  fi
}

run_command() {
  echo "$bin --config $config run --workspace-root $data/work $flags"
}

start_systemd() {
  if ! command -v systemctl >/dev/null 2>&1 || [ ! -d /run/systemd/system ]; then
    echo "no systemd: start the agent as $user with: $(run_command)"
    return
  fi
  $SUDO tee "$UNIT" >/dev/null <<EOF
[Unit]
Description=Scylla agent
Wants=network-online.target
After=network-online.target

[Service]
User=$user
ExecStart=$(run_command)
Restart=always
RestartSec=5
RestartPreventExitStatus=78

[Install]
WantedBy=multi-user.target
EOF
  $SUDO systemctl daemon-reload
  $SUDO systemctl enable --now scylla-agent
  echo "the agent runs: systemctl status scylla-agent, journalctl -u scylla-agent -f"
}

start_launchd() {
  mkdir -p "$(dirname "$plist")"
  {
    echo '<?xml version="1.0" encoding="UTF-8"?>'
    echo '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">'
    echo '<plist version="1.0"><dict>'
    echo "<key>Label</key><string>$LABEL</string>"
    echo '<key>ProgramArguments</key><array>'
    for argument in $(run_command); do
      echo "<string>$argument</string>"
    done
    echo '</array>'
    echo '<key>EnvironmentVariables</key><dict><key>PATH</key>'
    echo '<string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string></dict>'
    echo '<key>RunAtLoad</key><true/><key>KeepAlive</key><true/>'
    echo '<key>ThrottleInterval</key><integer>60</integer>'
    echo "<key>StandardOutPath</key><string>$data/agent.log</string>"
    echo "<key>StandardErrorPath</key><string>$data/agent.log</string>"
    echo '</dict></plist>'
  } >"$plist"
  attempt=0
  until launchctl bootstrap "$domain" "$plist" 2>/dev/null; do
    attempt=$((attempt + 1))
    [ "$attempt" -lt 5 ] || fail "launchd did not start the agent. The machine joined. Try: launchctl bootstrap $domain $plist"
    sleep 1
  done
  echo "the agent runs: launchctl print $domain/$LABEL, tail -f $data/agent.log"
}

start_service() {
  if [ "$service" = 0 ]; then
    echo "start the agent with: $(run_command)"
  elif [ "$os" = Linux ]; then
    start_systemd
  else
    start_launchd
  fi
}

install_agent() {
  if [ "$force" = 0 ] && $SUDO test -e "$config"; then
    fail "$config exists, so this machine joined before. --update replaces the binary and keeps the key. --force makes a new key"
  fi
  get_binary
  "$tmp/scylla-agent" --config "$tmp/agent.json" join "$url" "$code" >/dev/null ||
    fail "the join failed, and this machine did not change"
  echo "the machine joined $url"
  if [ "$force" = 1 ] && $SUDO test -e "$config"; then
    echo "the old key stays on the App until you revoke it"
  fi
  if [ -n "$user" ]; then
    make_user
  fi
  stop_service
  put_files
  start_service
}

update_agent() {
  $SUDO test -e "$config" || fail "$config does not exist, so this machine did not join. Give a join code"
  get_binary
  if [ -n "$user" ]; then
    $SUDO install -m 755 "$tmp/scylla-agent" "$bin"
  else
    install -m 755 "$tmp/scylla-agent" "$bin"
  fi
  if [ -n "$user" ] && [ -e "$UNIT" ]; then
    $SUDO systemctl restart scylla-agent
    echo "the agent runs $found"
  elif [ "$os" = Darwin ] && [ -e "$plist" ]; then
    launchctl kickstart -k "$domain/$LABEL"
    echo "the agent runs $found"
  else
    echo "$bin is now $found. Start the agent again"
  fi
}

uninstall_agent() {
  stop_service
  if [ -n "$user" ]; then
    if [ -e "$UNIT" ]; then
      $SUDO systemctl disable scylla-agent >/dev/null 2>&1 || true
      $SUDO rm -f "$UNIT"
      $SUDO systemctl daemon-reload
    fi
    $SUDO rm -f "$bin"
    $SUDO rm -rf "$data"
    if id "$user" >/dev/null 2>&1; then
      $SUDO userdel "$user"
    fi
  else
    if [ "$os" = Darwin ]; then
      rm -f "$plist"
    fi
    rm -rf "$data"
  fi
  echo "the agent left this machine. Its key stays on the server until you revoke it or delete the agent"
}

main() {
  parse "$@"
  find_target
  find_folders
  check_system
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  case "$action" in
    install) install_agent ;;
    update) update_agent ;;
    uninstall) uninstall_agent ;;
  esac
}

main "$@"
