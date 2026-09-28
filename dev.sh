#!/usr/bin/env bash
# Build on changes. Running games keep their current binary; the next launch
# uses the last successful build. Never put Cargo back in the launcher path.
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$project_dir"
if [[ "${1:-}" != --inside ]]; then
  exec nix develop "$project_dir" --command "$project_dir/dev.sh" --inside "$@"
fi
shift
# Lunchpail's linker flags must not create a second incompatible Cargo build.
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
if [[ "${1:-}" == --once ]]; then
  exec cargo build --workspace
fi
exec watchexec --on-busy-update queue --debounce 500ms --shell=none \
  --watch crates --watch Cargo.toml --watch Cargo.lock \
  -- cargo build --workspace
