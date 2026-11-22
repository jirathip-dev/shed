wire: code.grep
params: GrepParams
result: GrepResult
on: machine

The lines under `root` that match, grouped by the symbols around them, in name order.
The walk keeps to `.gitignore` and `info/exclude`, skips `.git`, dot names without `hidden`, symlinks and binary files.
A line over 4,096 bytes comes back `clipped` with its `length`.
Over 20,000 files, 250 rows, 256 MiB or 10 s is `LIMIT`.
`machine` searches there.
