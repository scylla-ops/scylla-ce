#!/usr/bin/env bash
# Checks the structure rules of AGENT.md that the compiler and clippy do not check.
set -uo pipefail
cd "$(dirname "$0")/.."

report() {
  local rule=$1 output
  output=$(cat)
  if [[ -n "$output" ]]; then
    echo "structure: $rule"
    echo "$output" | sed 's/^/  /'
  fi
}

rust_files() { find scylla-* -name '*.rs' -not -path '*/target/*'; }

checks() {
  grep -rnwiE 'quotas?|plans?|tenants?|enterprise' scylla-* proto \
    --include='*.rs' --include='*.proto' --include='*.sql' \
    | grep -vE '^[^:]+:[0-9]+:[[:space:]]*(//|--)' \
    | report "an EE word is in the code of a public crate"

  grep -rn 'cfg(feature' scylla-*/src | report "a cargo feature flag is in the code"

  rust_files | xargs wc -l | awk '$2 != "total" && $1 > 300 { print $2 ": " $1 " lines" }' \
    | report "a file has more than 300 lines"

  for file in $(rust_files); do
    head -n 1 "$file" | grep -q '^//!' || echo "$file"
  done | report "a Rust file does not start with a //! comment"

  # A proc macro must be in the lib.rs of its crate, so scylla-macros/src/lib.rs has its function.
  for file in $(find scylla-* \( -name mod.rs -o -name lib.rs \) -not -path '*/target/*' -not -path 'scylla-proto/*' -not -path 'scylla-macros/src/lib.rs'); do
    grep -vE '^[[:space:]]*(//|#\[)' "$file" | tr '\n' ' ' | tr ';' '\n' \
      | grep -vE '^[[:space:]]*$' \
      | grep -vE '^[[:space:]]*(pub(\(crate\))? )?(mod|use|extern crate) ' \
      | sed "s|^|$file: |"
  done | report "a mod.rs or lib.rs has more than mod and use lines"

  find scylla-*/src/features -name model.rs \
    | report "a feature folder has a model.rs (pure types go in the domain crate)"

  grep -n 'store::' scylla-*/src/features/*/grpc.rs | report "a grpc.rs calls a store"

  grep -nE 'tonic|scylla_proto' scylla-*/src/features/*/store.rs \
    | report "a store.rs knows tonic or the proto types"

  grep -rnE 'pub(\(crate\))? use scylla_domain' scylla-core/src \
    | report "a domain type is re-exported (import it from its domain crate)"

  grep -nE '^(sqlx|tonic|tokio|prost|async-trait|scylla-extension|scylla-core|scylla-proto)' \
    scylla-domain/Cargo.toml \
    | report "a domain crate has a heavy dependency"

  for manifest in scylla-*/Cargo.toml; do
    awk -v file="$manifest" '
      /^\[/ { deps = ($0 ~ /dependencies\]$/); next }
      deps && NF && $0 !~ /\.workspace = true$/ { print file ": " $0 }
    ' "$manifest"
  done | report "a crate sets a dependency version or features (use [workspace.dependencies])"

  rust_files | xargs grep -nE '^[[:space:]]*//([^/!]|$)' \
    | grep -vE ':[[:space:]]*// (given|when|then)$' \
    | report "a Rust file has a // comment (use /// or //!, or // given, when, then in a test)"

  grep -rln $'\xe2\x80\x94' --exclude-dir=target --exclude-dir=.idea --exclude-dir=.sqlx --exclude=Cargo.lock . \
    | report "a file has an em dash"
}

failures=$(checks)
if [[ -n "$failures" ]]; then
  echo "$failures"
  exit 1
fi
