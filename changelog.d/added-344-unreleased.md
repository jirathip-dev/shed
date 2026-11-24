- The `shed-secrets` crate, one secret store for shed, cooee and drover: Keychain items on macOS (a signed program
  only, every call limited to 5 s), 0600 files elsewhere (a wider one is not read). `shed secret set|ls|rm` and
  the `secret.*` methods keep shed's own; no method returns a value. `shed doctor` reports a wide file, a locked
  Keychain and an unsigned shed. Client tags carry the crate.
