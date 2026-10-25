#!/usr/bin/env bash
# Fails when the committed TypeScript bindings are not what the Rust DTOs produce.
#
# PLAN 0.4 requires the `export_bindings` pipeline and committed output under
# `web/src/api/gen/`. ts-rs regenerates those files whenever the export tests run, so a DTO that
# changed without a regenerated binding leaves the frontend compiling against a stale shape — the
# one class of defect the type generation exists to prevent, and one `cargo test` alone cannot
# catch because the files it rewrites are tracked. This runs the export tests and then requires the
# tracked output to be unchanged.
#
# The files are not reverted on failure: the regenerated copies are the fix, and the failure message
# says so.
set -euo pipefail

cd "$(dirname "$0")/.."

mkdir -p target/tmp
export TMPDIR="$PWD/target/tmp"

before="$(git status --porcelain web/src/api/gen)"
cargo test -p nekowriter export_bindings >/dev/null 2>&1
after="$(git status --porcelain web/src/api/gen)"

if [ "$before" != "$after" ]; then
    printf '\033[1;31m%s\033[0m\n' \
        "web/src/api/gen is stale: the export tests rewrote it."
    printf '%s\n' "$after"
    printf '%s\n' "Commit the regenerated files (they are already updated on disk)."
    exit 1
fi

echo "TypeScript bindings are up to date."
