`shed exec [--on M] [--cwd D] [--timeout S] [-i] -- CMD…` / `shed file read|write [--on M] PATH`

Run a program without a terminal and get exactly the bytes it wrote to stdout and stderr and its exit code; nothing it started outlives it.
`-i` feeds it stdin.
`file read` prints a file, `file write` replaces it with stdin (temp file and rename, mode kept).
A reply over 8 MiB is an error, never cut.
