#!/bin/sh
# shed-core depends on no other workspace crate; shed-script on shed-proto, shed-pattern and serde_json only.
set -eu
meta=$(cargo metadata --format-version 1 --no-deps --locked)
bad=$(echo "$meta" |
  jq -r '[.packages[].name] as $ws | .packages[] | select(.name == "shed-core") | .dependencies[].name | select(IN($ws[]))')
test -z "$bad" || { echo "shed-core must not depend on: $bad"; exit 1; }
echo "shed-core has no workspace deps"
bad=$(echo "$meta" |
  jq -r '.packages[] | select(.name == "shed-script") | .dependencies[] | select(.kind != "dev") | .name | select(IN("shed-proto", "shed-pattern", "serde_json") | not)')
test -z "$bad" || { echo "shed-script may depend only on shed-proto, shed-pattern and serde_json, not: $bad"; exit 1; }
echo "shed-script depends on shed-proto, shed-pattern and serde_json at most"
