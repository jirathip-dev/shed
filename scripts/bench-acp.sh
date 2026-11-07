#!/usr/bin/env bash
# The ACP benches (crates/shed/tests/acp_bench.rs) at the base and the head checkouts, interleaved release
# runs: the throughput bench (fake_acp.py streams 1,000 session/updates through the bridge to a socket
# client), median of 5, and the shed-only bench (10,000 canned updates; ns per update through the bridge
# and the server), median of 9. Fails if the head is more than 10% slower on either. Then the head's
# 1,000-update session/load replay, reported only. Both sides run the head's bench and fake agent. Builds
# go to $BENCH_TARGET/acp-{base,head}.
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
median() { printf '%s\n' "${@:2}" | sort -n | sed -n "$(($1 / 2 + 1))p"; }
# bench <test> <runs> <sed pattern for the number> <unit>: sets $slower when the head's median is over 110%.
slower=0
bench() {
  local b=() h=() out n
  for _ in $(seq "$2"); do
    for side in base head; do
      out=$(run "${!side}" "$side" "$1")
      n=$(echo "$out" | sed -n "$3")
      [ -n "$n" ] || { echo "$out"; exit 1; }
      echo "$side: $1 $n $4"
      if [ "$side" = base ]; then b+=("$n"); else h+=("$n"); fi
    done
  done
  local mb mh
  mb=$(median "$2" "${b[@]}") mh=$(median "$2" "${h[@]}")
  echo "acp $1 median of $2: base $mb $4, head $mh $4 (head/base $((mh * 100 / mb))%)"
  [ $((mh * 10)) -le $((mb * 11)) ] || { echo "FAIL: $1: the head is more than 10% slower"; slower=1; }
}
for side in base head; do run "${!side}" "$side" throughput >/dev/null; done
bench throughput 5 's/^acp throughput: \([0-9]*\) us$/\1/p' us
bench shed_only 9 's/^acp shed-only: \([0-9]*\) ns per update.*/\1/p' ns
run "$head" head replay | grep '^acp replay:'
exit "$slower"
