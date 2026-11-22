- `shed code grep [-F] [-i] [-w] [--count] [--hidden] [-g GLOB]… PATTERN [PATH]` (`code.grep`, with `machine`
  over a held link): the matching lines under a folder, grouped by the function or type that holds each one,
  files in name order. The walk keeps to `.gitignore` (the root's, every folder's, the folders' above it up to
  the repository's top) and `info/exclude` (also through a worktree's `.git` file), skips `.git`, dot names
  without `--hidden`, symlinks and binary files. Over 20,000 files, 250 rows, 256 MiB or 10 s is `LIMIT`; a
  matched line over 4,096 bytes comes back `clipped` with its full `length`.
