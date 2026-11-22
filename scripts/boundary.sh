#!/bin/sh
# No drover or cooee job in shed (#257): crates/*/src under ROOT (default .), up to a file's first `#[cfg(test)]`.
# Workflow (issues, pull requests, review, merge) belongs to drover; bots and chat to cooee.
set -eu
root=${1:-.}
found=$(find "$root"/crates/*/src -name '*.rs' -exec awk \
  '/^[[:space:]]*#\[cfg\(test\)\]/ { exit } tolower($0) ~ /pull_request|\/pulls|\/issues|gh (pr|issue)|telegram|slack|discord|screens\.json/ { print FILENAME ":" FNR ": " $0 }' {} \;)
if [ -n "$found" ]; then
  echo "$found"
  echo "workflow or chat code in shed; it belongs to drover (issues, queue, review, merge) or cooee (bots, chat)"
  exit 1
fi
echo "shed holds no workflow or chat code"
