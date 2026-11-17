#!/bin/sh
# No agent or provider name in core code (#64): crates/*/src under ROOT (default .), outside providers/,
# up to a file's first `#[cfg(test)]`. Agent specifics are data: crates/shed-server/src/providers/<name>.toml.
set -eu
root=${1:-.}
found=$(find "$root"/crates/*/src -name '*.rs' ! -path '*/providers/*' -exec awk \
  '/^[[:space:]]*#\[cfg\(test\)\]/ { exit } tolower($0) ~ /claude|anthropic|codex|openai|gemini|hermes/ { print FILENAME ":" FNR ": " $0 }' {} \;)
if [ -n "$found" ]; then
  echo "$found"
  echo "agent or provider names in core code; move them to the data file crates/shed-server/src/providers/<name>.toml"
  exit 1
fi
echo "core names no agent or provider"
