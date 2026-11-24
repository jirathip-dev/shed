- A release of a main commit that lacks `scripts/fake-link.py` no longer fails: staging's release workflow skips
  `shed-fake-link.py` with a notice and still publishes the binaries.
