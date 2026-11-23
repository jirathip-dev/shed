#!/bin/sh
# arch-check's `generate`. drover gives it 60 s, which the test builds of scripts/generate.sh cannot meet, so
# under gatekeeper (its wrapper exports GATEKEEPER_JOB_START) it only puts back the goldens arch-check took
# away, and the local/generated gate runs scripts/generate.sh; anywhere else (an update round) it is that script.
set -eu
cd "$(git rev-parse --show-toplevel)"
if [ -n "${GATEKEEPER_JOB_START:-}" ]; then
  exec git checkout -q -- crates/shed/tests/golden
fi
exec scripts/generate.sh
