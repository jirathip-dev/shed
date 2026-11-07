#!/usr/bin/env bash
# The ACP throughput bench (crates/shed/tests/acp_bench.rs: fake_acp.py streams 1,000 session/updates
# through the bridge to a socket client) at the base and the head checkouts: the median of 5 interleaved
# release runs each. Fails if the head is more than 10% slower. Then the head's 1,000-update session/load
# replay and the hibernation bench (10 idle fake agents' RSS with hibernation off and on, failing unless
# it is lower on; the wake time), reported only. Both sides run the head's bench and fake agent. Builds go to
# $BENCH_TARGET/acp-{base,head}.
set -euo pipefail
base=$(cd "$1" && pwd) head=$(cd "$2" && pwd)
target=${BENCH_TARGET:-$head/target}
for f in crates/shed/tests/acp_bench.rs crates/shed/tests/fake_acp.py; do
  [ "$base" = "$head" ] || cp "$head/$f" "$base/$f"
done
run() {
  (cd "$1" && CARGO_TARGET_DIR="$target/acp-$2" cargo test -q -p shed --release --locked --test acp_bench \
    "$3" -- --ignored --exact --nocapture)
}
median() { printf '%s\n' "$@" | sort -n | sed -n 3p; }
b=() h=()
for side in base head; do run "${!side}" "$side" throughput >/dev/null; done
for _ in 1 2 3 4 5; do
  for side in base head; do
    out=$(run "${!side}" "$side" throughput)
    us=$(echo "$out" | sed -n 's/^acp throughput: \([0-9]*\) us$/\1/p')
    [ -n "$us" ] || { echo "$out"; exit 1; }
    echo "$side: acp throughput $us us"
    if [ "$side" = base ]; then b+=("$us"); else h+=("$us"); fi
  done
done
mb=$(median "${b[@]}") mh=$(median "${h[@]}")
echo "acp throughput median of 5: base $mb us, head $mh us (head/base $((mh * 100 / mb))%)"
run "$head" head replay | grep '^acp replay:'
run "$head" head hibernate | grep '^acp hibernate'
if [ $((mh * 10)) -gt $((mb * 11)) ]; then
  echo "FAIL: the head is more than 10% slower than the base"
  exit 1
fi
