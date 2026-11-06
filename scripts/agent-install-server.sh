#!/usr/bin/env bash
# Starts a server for agent-install-test.sh: new token keys, the migrations, and the server on
# SERVER. The server writes to server.log, and its environment comes from the caller.
set -euo pipefail

keys=$("$SCYLLA_CE" token keygen 2>/dev/null | tr -d '\r')
SCYLLA_TOKEN_PRIVATE_KEY=$(sed -n 's/^private: //p' <<<"$keys")
SCYLLA_TOKEN_PUBLIC_KEYS=$(sed -n 's/^public: //p' <<<"$keys")
export SCYLLA_TOKEN_PRIVATE_KEY SCYLLA_TOKEN_PUBLIC_KEYS
if [ -n "${GITHUB_ENV:-}" ]; then
  echo "::add-mask::$SCYLLA_TOKEN_PRIVATE_KEY"
  {
    echo "SCYLLA_TOKEN_PRIVATE_KEY=$SCYLLA_TOKEN_PRIVATE_KEY"
    echo "SCYLLA_TOKEN_PUBLIC_KEYS=$SCYLLA_TOKEN_PUBLIC_KEYS"
  } >>"$GITHUB_ENV"
fi

for _ in $(seq 60); do
  if "$SCYLLA_CE" migrate >/dev/null 2>&1; then
    break
  fi
  sleep 2
done
"$SCYLLA_CE" migrate
nohup "$SCYLLA_CE" serve >server.log 2>&1 &
for _ in $(seq 60); do
  if curl -fsS "$SERVER/install.sh" >/dev/null 2>&1; then
    echo "the server runs on $SERVER"
    exit 0
  fi
  sleep 1
done
tail -n 50 server.log
exit 1
