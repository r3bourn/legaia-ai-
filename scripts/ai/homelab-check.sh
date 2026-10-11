#!/usr/bin/env bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

AI_PACKAGES=(
  legaia-ai-controller
  legaia-ai-guide
  legaia-ai-navigation
  legaia-ai-battle
  legaia-ai-orchestrator
  legaia-ai-broker
)

printf '\n== Legaia AI foundation verification ==\n'
printf 'repo: %s\n' "$(pwd)"
printf 'rustc: '; rustc --version
printf 'cargo: '; cargo --version

printf '\n[1/5] rustfmt check\n'
cargo fmt --all -- --check

printf '\n[2/5] native type-check\n'
check_args=()
for package in "${AI_PACKAGES[@]}"; do
  check_args+=(-p "$package")
done
cargo check "${check_args[@]}" -p legaia-web-viewer

printf '\n[3/5] AI crate tests\n'
for package in "${AI_PACKAGES[@]}"; do
  printf '\n-- %s --\n' "$package"
  cargo test -p "$package" --profile release-test
done

printf '\n[4/5] browser crate native tests/type-check\n'
cargo test -p legaia-web-viewer --profile release-test

printf '\n[5/5] wasm32 type-check\n'
if ! rustup target list --installed | grep -qx 'wasm32-unknown-unknown'; then
  cat >&2 <<'EOF'
wasm32-unknown-unknown is not installed.
Install it once with:
  rustup target add wasm32-unknown-unknown
Then rerun this script.
EOF
  exit 2
fi
cargo check -p legaia-web-viewer --target wasm32-unknown-unknown

printf '\nPASS: targeted AI + browser verification completed.\n'
