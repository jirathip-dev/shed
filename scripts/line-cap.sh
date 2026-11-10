#!/bin/sh
# Non-test lines across crates/*/src must stay at most line_cap in ci/cap.toml.
# A line is non-test when it is not blank and comes before the file's first `#[cfg(test)]`
# (unit tests live at the end of a file).
set -eu
cap=$(sed -n 's/^line_cap *= *\([0-9]*\).*/\1/p' ci/cap.toml)
n=$(find crates/*/src -name '*.rs' -exec awk '/^[[:space:]]*#\[cfg\(test\)\]/ { exit } NF { n++ } END { print n + 0 }' {} \; |
  awk '{ s += $1 } END { print s + 0 }')
echo "$n / cap $cap"
test "$n" -le "$cap"
