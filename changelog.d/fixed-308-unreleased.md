- One hung machine link no longer blocks the server: a quick call over a link (`ls`, `get`, `read`, `kill`,
  `state`, …) answers or fails `NO_LINK` within 10 s (an `exec` within its own timeout plus 10 s), and a link
  whose call got no answer is cut, so `machine ls` reads it reconnecting at once. `ls` asks every machine at
  the same time. A link's handshake gives up after 20 s instead of waiting for ever.
- `scripts/fake-link.py`, a fake `ssh` with latency, loss, stalls, hangs, cuts, a host gone, an address change
  and a host-key change, ships with each release as `shed-fake-link.py`; shed's machine tests run over it.
  `scripts/soak.py` times throwaway terminals on real machines against `ci/soak.toml`, nightly.
