- `shed self-update` follows main's releases: it reads main's head, downloads that commit's `build-<sha7>` asset,
  checks it against `SHA256SUMS` and its `--version`, asks serve whether the new build can take over, swaps the
  binary and hands serve over without dropping terminals. No release yet, a bad checksum, another build or a
  refused check leave the binary and serve as they were; staging releases are never installed. `--dry-run`,
  `--pause` and `--resume`. `shed install` also installs the 5-minute job (`dev.shed.update`, or the
  `shed-update.timer` user timer), and `shed doctor` has an `update` row. `shed wait` and `shed events`
  reconnect when a handover closes their connection.
