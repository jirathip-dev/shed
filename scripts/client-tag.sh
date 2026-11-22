#!/bin/sh
# The client release tag for a commit (#267): client-v<PROTOCOL>.<n> when crates/shed-proto, shed-client or
# shed-catalog changed since the newest client-v* tag reachable from it. Prints the tag, or why there is none;
# with --push also pushes it to origin. Usage: scripts/client-tag.sh [--push] [<commit>] (default HEAD).
set -eu
push=
if [ "${1:-}" = --push ]; then push=1; shift; fi
sha=$(git rev-parse --verify "${1:-HEAD}^{commit}")
p=$(git show "$sha:crates/shed-proto/src/lib.rs" | sed -n 's/^pub const PROTOCOL: u32 = \([0-9]*\);$/\1/p')
[ -n "$p" ] || { echo "client-tag: no PROTOCOL in crates/shed-proto/src/lib.rs at $sha" >&2; exit 1; }
last=$(git tag --list 'client-v*' --merged "$sha" --sort=-v:refname | head -n 1)
if [ -n "$last" ] && git diff --quiet "$last" "$sha" -- crates/shed-proto crates/shed-client crates/shed-catalog; then
  echo "no client change since $last"
  exit 0
fi
n=$(git tag --list "client-v$p.*" --sort=-v:refname | head -n 1 | sed 's/.*\.//')
tag="client-v$p.$((${n:--1} + 1))"
echo "$tag $sha"
[ -z "$push" ] || git push origin "$sha:refs/tags/$tag"
