#!/bin/sh
# The generated files in .drover/arch.toml: CHANGELOG.md's Unreleased section from changelog.d/, then
# docs/protocol.md and docs/shed.openrpc.json, then the script engine's counts of its fixed scripts
# (crates/shed-script/tests/perf/counts.json), then the goldens the CLI, ACP, server and machines tests write
# (crates/shed/tests/golden/, their `golden.rs`), then Cargo.lock from the manifests, then README.md's command
# table from docs/commands/.
set -eu
cd "$(dirname "$0")/.."
python3 -I scripts/changelog.py
SHED_BLESS=1 cargo test -p shed-proto --features schema --locked --test protocol_doc
SHED_BLESS=1 cargo test -p shed-script --locked --test perf counts
SHED_BLESS=1 cargo test -p shed --locked --test cli --test acp --test server --test machines golden::
cargo metadata --format-version 1 > /dev/null
python3 -I scripts/commands.py
