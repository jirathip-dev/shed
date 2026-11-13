#!/bin/sh
# Each ci/fixtures/banned/<lint>.rs (or DIR/<lint>.rs) breaks one banned-API lint; clippy must reject it,
# naming that lint, under the workspace's own [workspace.lints] and clippy.toml.
set -eu
crate=${CARGO_TARGET_DIR:-target}/banned-fixture
mkdir -p "$crate/src"
{
  printf '[package]\nname = "banned"\nversion = "0.0.0"\nedition = "2024"\n\n[lints]\nworkspace = true\n\n[workspace]\n\n'
  awk '/^\[/ { p = /^\[workspace\.lints/ } p' Cargo.toml
} > "$crate/Cargo.toml"
cp clippy.toml "$crate/"
for f in "${1:-ci/fixtures/banned}"/*.rs; do
  lint=$(basename "$f" .rs)
  cp "$f" "$crate/src/lib.rs"
  if out=$(cargo clippy --quiet --manifest-path "$crate/Cargo.toml" -- -D warnings 2>&1); then
    echo "$f: clippy passed; it must reject $lint"
    exit 1
  fi
  if ! printf '%s' "$out" | grep -qE "with \`-[DF] (clippy::)?$(echo "$lint" | tr _ -)\`"; then
    printf '%s\n%s: clippy failed, but not on %s\n' "$out" "$f" "$lint"
    exit 1
  fi
  echo "$f: rejected ($lint)"
done
