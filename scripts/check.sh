#!/usr/bin/env bash
# All checks, in the order fixed by docs/spec/01-architecture.md §12. Fails fast.
set -euo pipefail

cd "$(dirname "$0")/.."

# Tests use temporary databases and fixtures; keep them inside the repository (the cargo config
# sets TMPDIR for cargo-run processes, and this covers the pnpm steps too).
mkdir -p target/tmp
export TMPDIR="$PWD/target/tmp"

step() {
    printf '\n\033[1m==> %s\033[0m\n' "$1"
}

step "Prompts"
python3 -I scripts/validate_prompts.py .

step "cargo fmt"
cargo fmt --all --check

step "cargo clippy"
cargo clippy --workspace --all-targets -- -D warnings

step "cargo test"
cargo test --workspace

step "web typecheck"
pnpm --dir web typecheck

step "web lint"
pnpm --dir web lint

step "web test"
pnpm --dir web test --run

step "web i18n"
pnpm --dir web i18n:check

step "web build"
pnpm --dir web build

printf '\n\033[1;32mAll checks passed.\033[0m\n'
