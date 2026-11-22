#!/bin/sh
# Non-test lines across crates/*/src (under DIR, default .) must stay at most line_cap: the newest raise
# in ci/cap.d/ by file name, else ci/cap.toml.
# A line is non-test when it is not blank and comes before the file's first `#[cfg(test)]`
# (unit tests live at the end of a file).
set -eu
cd "${1:-.}"
src=ci/cap.toml
for f in ci/cap.d/*.toml; do
  if [ -e "$f" ]; then src=$f; fi
done
cap=$(sed -n 's/^line_cap *= *\([0-9]*\).*/\1/p' "$src")
n=$(find crates/*/src -name '*.rs' -exec awk '/^[[:space:]]*#\[cfg\(test\)\]/ { exit } NF { n++ } END { print n + 0 }' {} \; |
  awk '{ s += $1 } END { print s + 0 }')
echo "$n / cap $cap"
test "$n" -le "$cap"
