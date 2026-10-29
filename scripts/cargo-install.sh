#!/bin/sh
# Run `cargo install` with its scratch directory inside the repository.
#
# Cargo reads TMPDIR at startup, before `.cargo/config.toml`'s `[env]` table applies, so a bare
# `cargo install` builds under /tmp. On this machine /tmp is a 7.5 GB tmpfs; a single
# `cargo install sea-orm-cli` filled it and took every process spawn on the machine down with it.
#
#   scripts/cargo-install.sh sea-orm-cli@2.0.4 --locked --no-default-features \
#     --features codegen,sqlx-sqlite,runtime-tokio --root target/tmp/seaorm-cli
set -eu
root=$(git rev-parse --show-toplevel)
scratch="$root/target/tmp/cargo-install"
mkdir -p "$scratch/tmp" "$scratch/build"
TMPDIR="$scratch/tmp" CARGO_TARGET_DIR="$scratch/build" exec cargo install "$@"
