#!/bin/sh
# shed-proto with default features pulls in neither serde_json's preserve_order nor schemars (#123),
# so a user's JSON keeps sorted keys; with `schema` both must show, which proves the check bites.
set -eu
tree() { cargo tree -p shed-proto -e features --locked "$@"; }
leaks() { grep -E 'serde_json feature "preserve_order"|^.*schemars v' || true; }
bad=$(tree | leaks)
test -z "$bad" || { echo "shed-proto default features leak:"; echo "$bad"; exit 1; }
test -n "$(tree --features schema | leaks)" || { echo "the check found nothing even with --features schema"; exit 1; }
echo "shed-proto default features: no preserve_order, no schemars"
