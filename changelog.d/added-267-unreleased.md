- Client releases: after a green push to main the release workflow tags `client-v<PROTOCOL>.<n>` when
  `shed-proto`, `shed-client` or `shed-catalog` changed since the last such tag, so clients can pin shed's client
  crates by tag (docs/releases.md). Starting the workflow by hand is a dry run that prints the tag it would push.
