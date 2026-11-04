#!/bin/sh
# Installs the Scylla agent on Linux or macOS, joins its App, and starts it as a service.
#
#   curl -fsSL <server>/install.sh | sh -s -- <join-code> [options]
#
# Options:
#   --capacity <n>              the largest number of jobs at the same time (default 1)
#   --container-runtime <name>  docker, podman, or none (default: docker or podman if found)
#   --no-service                install and join only; print the command that runs the agent
#
# SCYLLA_AGENT_DOWNLOADS_URL replaces the folder of the downloads.
set -eu

SCYLLA_URL='@SCYLLA_URL@'
VERSION='@SCYLLA_VERSION@'
DEFAULT_DOWNLOADS='@DOWNLOADS_URL@'
DOWNLOADS="${SCYLLA_AGENT_DOWNLOADS_URL:-$DEFAULT_DOWNLOADS}"
BIN=/usr/local/bin/scylla-agent
HOME_DIR=/var/lib/scylla-agent
CONFIG="$HOME_DIR/agent.json"
LABEL=com.scylla.agent

fail() {
  echo "scylla-agent install: $*" >&2
  exit 1
}

code=""
capacity=1
runtime=auto
service=1
while [ $# -gt 0 ]; do
  case "$1" in
    --capacity)
      [ $# -ge 2 ] || fail "--capacity needs a number"
      capacity=$2
      shift 2
      ;;
    --container-runtime)
      [ $# -ge 2 ] || fail "--container-runtime needs a name"
      runtime=$2
      shift 2
      ;;
    --no-service)
      service=0
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
[ -n "$code" ] || fail "give the join code of the app: sh -s -- <join-code>"
case "$capacity" in '' | *[!0-9]* | 0) fail "--capacity needs a number from 1" ;; esac
case "$runtime" in '' | *[!a-z0-9_-]*) fail "--container-runtime needs a name, for example docker" ;; esac

if [ "$(id -u)" -eq 0 ]; then
  SUDO=""
else
  command -v sudo >/dev/null 2>&1 || fail "run as root, or install sudo"
  SUDO=sudo
fi

arch=$(uname -m)
case "$arch" in
  x86_64 | amd64) arch=x86_64 ;;
  aarch64 | arm64) arch=aarch64 ;;
  *) fail "no build for the CPU $arch" ;;
esac
os=$(uname -s)
case "$os" in
  Linux) target="$arch-unknown-linux-musl" ;;
  Darwin) target="$arch-apple-darwin" ;;
  *) fail "no build for $os; on Windows, use install.ps1" ;;
esac

if [ "$runtime" = auto ]; then
  runtime=none
  for candidate in docker podman; do
    if command -v "$candidate" >/dev/null 2>&1; then
      runtime=$candidate
      break
    fi
  done
fi

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

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
archive="scylla-agent-$target.tar.gz"
echo "downloading scylla-agent $VERSION for $target"
download "$DOWNLOADS/$archive" "$tmp/$archive" || fail "the download of $DOWNLOADS/$archive failed"
download "$DOWNLOADS/SHA256SUMS" "$tmp/SHA256SUMS" || fail "the download of the checksums failed"
expected=$(awk -v file="$archive" '$2 == file || $2 == "*" file { print $1 }' "$tmp/SHA256SUMS")
[ -n "$expected" ] || fail "no checksum for $archive"
[ "$expected" = "$(sha256 "$tmp/$archive")" ] || fail "the checksum of $archive does not match"
tar -xzf "$tmp/$archive" -C "$tmp"

if [ "$os" = Linux ]; then
  user=scylla-agent
  if ! id "$user" >/dev/null 2>&1; then
    command -v useradd >/dev/null 2>&1 || fail "install useradd, or make the user $user"
    $SUDO useradd --system --home-dir "$HOME_DIR" --shell /usr/sbin/nologin "$user"
  fi
  if [ "$runtime" = docker ] && getent group docker >/dev/null 2>&1; then
    $SUDO usermod -aG docker "$user"
  fi
  if [ "$service" = 1 ] && command -v systemctl >/dev/null 2>&1; then
    $SUDO systemctl stop scylla-agent >/dev/null 2>&1 || true
  fi
else
  user=${SUDO_USER:-$(id -un)}
  if [ "$service" = 1 ]; then
    $SUDO launchctl bootout "system/$LABEL" >/dev/null 2>&1 || true
  fi
fi

$SUDO mkdir -p /usr/local/bin "$HOME_DIR/work"
$SUDO install -m 0755 "$tmp/scylla-agent" "$BIN"
$SUDO "$BIN" --config "$CONFIG" join "$SCYLLA_URL" "$code"
$SUDO chown -R "$user" "$HOME_DIR"
$SUDO chmod 700 "$HOME_DIR"
$SUDO chmod 600 "$CONFIG"

set -- --config "$CONFIG" run --workspace-root "$HOME_DIR/work" --capacity "$capacity"
if [ "$runtime" != none ]; then
  set -- "$@" --container-runtime "$runtime"
fi

if [ "$service" = 0 ]; then
  echo "start the agent as $user with: $BIN $*"
  exit 0
fi

if [ "$os" = Linux ]; then
  if ! command -v systemctl >/dev/null 2>&1 || [ ! -d /run/systemd/system ]; then
    echo "no systemd: start the agent as $user with: $BIN $*"
    exit 0
  fi
  $SUDO tee /etc/systemd/system/scylla-agent.service >/dev/null <<EOF
[Unit]
Description=Scylla agent
Wants=network-online.target
After=network-online.target

[Service]
User=$user
ExecStart=$BIN $*
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF
  $SUDO systemctl daemon-reload
  $SUDO systemctl enable --now scylla-agent
  echo "the agent runs: systemctl status scylla-agent, journalctl -u scylla-agent -f"
else
  plist="/Library/LaunchDaemons/$LABEL.plist"
  {
    echo '<?xml version="1.0" encoding="UTF-8"?>'
    echo '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">'
    echo '<plist version="1.0"><dict>'
    echo "<key>Label</key><string>$LABEL</string>"
    echo "<key>UserName</key><string>$user</string>"
    echo '<key>ProgramArguments</key><array>'
    echo "<string>$BIN</string>"
    for argument in "$@"; do
      echo "<string>$argument</string>"
    done
    echo '</array>'
    echo '<key>EnvironmentVariables</key><dict><key>PATH</key>'
    echo '<string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string></dict>'
    echo '<key>RunAtLoad</key><true/><key>KeepAlive</key><true/>'
    echo "<key>StandardErrorPath</key><string>$HOME_DIR/agent.log</string>"
    echo '</dict></plist>'
  } | $SUDO tee "$plist" >/dev/null
  $SUDO launchctl bootstrap system "$plist"
  echo "the agent runs: sudo launchctl print system/$LABEL, logs in $HOME_DIR/agent.log"
fi
