#!/usr/bin/env bash
# The event fan-out bench (crates/shed-server/tests/bench/fanout.rs) at the base and the head checkouts:
# the median of 5 interleaved release runs each. Fails if the head is more than 10% slower.
# A base without the bench gets the head's. Builds go to $BENCH_TARGET/fanout-{base,head}. Then, report
# only, 500 idle events clients and one state change at each side (crates/shed/tests/idle_bench.rs).
set -euo pipefail
base=$(cd "$1" && pwd) head=$(cd "$2" && pwd)
target=${BENCH_TARGET:-$head/target}
bench=crates/shed-server/tests/bench/fanout.rs
if [ ! -f "$base/$bench" ]; then
  mkdir -p "$(dirname "$base/$bench")"
  cp "$head/$bench" "$base/$bench"
  printf '\n#[cfg(test)]\n#[path = "../tests/bench/fanout.rs"]\nmod bench;\n' >>"$base/crates/shed-server/src/events.rs"
fi
run() {
  (cd "$1" && CARGO_TARGET_DIR="$target/fanout-$2" cargo test -q -p shed-server --release --locked --lib \
    events::bench::fanout -- --ignored --exact --nocapture)
}
median() { printf '%s\n' "$@" | sort -n | sed -n 3p; }
b=() h=()
for side in base head; do run "${!side}" "$side" >/dev/null; done
for _ in 1 2 3 4 5; do
  for side in base head; do
    out=$(run "${!side}" "$side")
    echo "$side: $(echo "$out" | grep '^fanout n=' | tr '\n' ' ')"
    us=$(echo "$out" | sed -n 's/^fanout total: \([0-9]*\) us$/\1/p')
    [ -n "$us" ] || { echo "$out"; exit 1; }
    if [ "$side" = base ]; then b+=("$us"); else h+=("$us"); fi
  done
done
idle=crates/shed/tests/idle_bench.rs
[ -f "$base/$idle" ] || cp "$head/$idle" "$base/$idle"
for side in base head; do
  (cd "${!side}" && ulimit -n 4096 && CARGO_TARGET_DIR="$target/fanout-$side" cargo test -q -p shed --release \
    --locked --test idle_bench -- --ignored --nocapture | sed -n "s/^idle /$side: idle /p") || echo "$side: idle failed"
done
mb=$(median "${b[@]}") mh=$(median "${h[@]}")
echo "fan-out median of 5: base $mb us, head $mh us (head/base $((mh * 100 / mb))%)"
if [ $((mh * 10)) -gt $((mb * 11)) ]; then
  echo "FAIL: the head is more than 10% slower than the base"
  exit 1
fi
