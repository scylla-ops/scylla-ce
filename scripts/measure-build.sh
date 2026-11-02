#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

now() { perl -MTime::HiRes=time -e 'printf "%.2f\n", time'; }

host=$(rustc -vV | sed -n 's/^host: //p')
flags_var="CARGO_TARGET_$(echo "$host" | tr 'a-z-' 'A-Z_')_RUSTFLAGS"

edits=(
  "domain|scylla-domain/src/projects.rs|MAX_LEN: usize = 100;|MAX_LEN: usize = 101;"
  "store (SQL)|scylla-core/src/features/projects/store.rs|ORDER BY id|ORDER BY id ASC"
  "grpc handler|scylla-core/src/features/projects/grpc.rs|\"page_token\")|\"page_token.\")"
)

backup=""
restore() { if [[ -n "$backup" ]]; then mv "$backup" "${backup%.measure-bak}" && touch "${backup%.measure-bak}"; backup=""; fi; }
trap restore EXIT

timed_build() {
  local start end
  start=$(now)
  cargo build --workspace --quiet
  end=$(now)
  perl -e "printf '%.1f', $end - $start"
}

measure() {
  local linker=$1
  export CARGO_TARGET_DIR="target/measure-$linker"
  if [[ "$linker" == lld ]]; then
    export "$flags_var=-C link-arg=-fuse-ld=lld"
  else
    unset "$flags_var" || true
  fi

  rm -rf "$CARGO_TARGET_DIR"
  echo "| $linker | cold build | $(timed_build) s |"

  for edit in "${edits[@]}"; do
    IFS='|' read -r layer file from to <<<"$edit"
    backup="$file.measure-bak"
    cp "$file" "$backup"
    perl -pi -e "s/\Q$from\E/$to/" "$file"
    cmp -s "$file" "$backup" && { echo "edit did not apply: $layer" >&2; exit 1; }
    echo "| $linker | one line in $layer | $(timed_build) s |"
    restore
    cargo build --workspace --quiet
  done
}

echo "| linker | change | cargo build --workspace |"
echo "|---|---|---|"
measure default
if command -v ld64.lld >/dev/null || command -v ld.lld >/dev/null; then
  measure lld
fi
