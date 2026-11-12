#!/usr/bin/env bash
# The drain bench (crates/shed/tests/drain_bench.rs) at the base and the head checkouts: each stream
# (plain, colour, redraw; $SHED_DRAIN_MB MB, default 100) `cat` into a fresh serve's terminal, timed until
# `read` shows its last line, with serve's peak RSS; median of 5 interleaved release runs. Fails if the
# head is more than 10% slower on any stream. Then the raw-read ceiling at each side
# (crates/shed-core/tests/drain_raw.rs), reported only. Both sides run the head's bench. Builds go to
# $BENCH_TARGET/drain-{base,head}.
set -euo pipefail
base=$(cd "$1" && pwd) head=$(cd "$2" && pwd)
target=${BENCH_TARGET:-$head/target}
for f in crates/shed/tests/drain_bench.rs crates/shed/tests/drain/fixture.rs crates/shed-core/tests/drain_raw.rs; do
  [ "$base" = "$head" ] || { mkdir -p "$(dirname "$base/$f")" && cp "$head/$f" "$base/$f"; }
done
run() {
  (cd "${!1}" && CARGO_TARGET_DIR="$target/drain-$1" cargo test -q -p "$2" --release --locked --test "$3" \
    -- --ignored --exact --nocapture "${@:4}")
}
median() { printf '%s\n' "$@" | sort -n | sed -n 3p; }
mkdir -p "$target"
out="$target/drain.log"
: >"$out"
for _ in 1 2 3 4 5; do
  for side in base head; do
    run "$side" shed drain_bench drain_streams | grep '^drain ' | sed "s/^/$side: /" | tee -a "$out"
  done
done
for side in base head; do
  run "$side" shed-core drain_raw drain_raw | grep '^drain raw ' | sed "s/^/$side: /"
done
slower=0
for kind in plain colour redraw; do
  ms() { sed -n "s/^$1: drain $kind: \([0-9]*\) ms.*/\1/p" "$out"; }
  rss() { sed -n "s/^$1: drain $kind peak rss: \([0-9]*\) MB/\1/p" "$out"; }
  # shellcheck disable=SC2046
  mb=$(median $(ms base)) mh=$(median $(ms head)) rb=$(median $(rss base)) rh=$(median $(rss head))
  echo "drain $kind median of 5: base $mb ms, head $mh ms (head/base $((mh * 100 / mb))%); peak rss base $rb MB, head $rh MB"
  [ $((mh * 10)) -le $((mb * 11)) ] || { echo "FAIL: $kind: the head is more than 10% slower"; slower=1; }
done
exit "$slower"
