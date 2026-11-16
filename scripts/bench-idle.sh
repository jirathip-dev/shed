#!/bin/sh
# Threads and RSS of `shed serve` with N idle terminals (#131): bench-idle.sh <shed binary> <N>.
# Fails unless `ls` lists all N and serve runs at most 3 threads per terminal plus 40.
set -eu
shed=$1
n=$2
dir=$(mktemp -d)
trap 'kill "$(cat "$dir/serve.pid")" 2>/dev/null; wait; rm -rf "${dir:?}"' EXIT
"$shed" serve --dir "$dir" >/dev/null 2>&1 &
echo $! >"$dir/serve.pid"
pid=$(cat "$dir/serve.pid")
until "$shed" --dir "$dir" ls >/dev/null 2>&1; do sleep 0.1; done
i=0
while [ "$i" -lt "$n" ]; do
	i=$((i + 1))
	"$shed" --dir "$dir" run --name "t$i" -- sleep 600 >/dev/null
done
"$shed" --dir "$dir" ls >"$dir/ls.txt"
listed=$(grep -c '^t[0-9]' "$dir/ls.txt" || true)
sleep 2
if [ "$(uname)" = Darwin ]; then
	threads=$(($(ps -M -p "$pid" | wc -l) - 1))
else
	threads=$(ps -o nlwp= -p "$pid" | tr -d ' ')
fi
rss=$(ps -o rss= -p "$pid" | tr -d ' ')
echo "terminals=$n listed=$listed threads=$threads rss_kb=$rss"
[ "$listed" = "$n" ]
[ "$threads" -le $((3 * n + 40)) ]
