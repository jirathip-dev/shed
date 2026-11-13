#!/bin/sh
# shed-core depends on no other workspace crate.
set -eu
bad=$(cargo metadata --format-version 1 --no-deps --locked |
  jq -r '[.packages[].name] as $ws | .packages[] | select(.name == "shed-core") | .dependencies[].name | select(IN($ws[]))')
test -z "$bad" || { echo "shed-core must not depend on: $bad"; exit 1; }
echo "shed-core has no workspace deps"
