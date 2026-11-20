#!/bin/sh
# Tests leave no temp folders: none named shed-* in DIR (default target/test-tmp, the tests' own temp dir) after `just test`.
set -eu
dir=${1:-target/test-tmp/}
left=$(ls -a "$dir" | grep '^shed-' || true)
if [ -n "$left" ]; then
  echo "$left"
  echo "tests left these in $dir; make each test remove its temp folders"
  exit 1
fi
echo "tests left no temp folders"
