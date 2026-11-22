# Contributing

## Run the gates locally

```sh
./.gatekeeper/run check
```

It runs every CI gate through the pinned [gatekeeper](https://github.com/jirathip-dev/gatekeeper), which fetches
itself and its tools: its core and rust packs (fmt, clippy, the banned APIs, cargo-deny, the line cap, arch-check,
hygiene, actionlint) and shed's own gates in `.gatekeeper/gates/` (tests, docs, generated files, benches). One gate
runs alone with `--gate`, e.g. `./.gatekeeper/run check --gate rust/clippy`; `./.gatekeeper/run list` lists them.
The toolchain is the MSRV pinned in `rust-toolchain.toml`; CI also tests stable.

## Pull requests

- Open the PR against `staging`; fill in the template, including `## For humans` and the line count.
- New behaviour comes with a test that fails without it.
- Placeholders only in docs, tests and fixtures (`example.com`, `host-a`, `/home/user`).
- No AI co-author or "generated with" trailers.
- Note user-visible changes in a new `changelog.d/<kind>-<issue>.md` (kind `added`, `changed`, `fixed`,
  `removed`, `deprecated` or `security`; one or more `- ` bullets), never in `CHANGELOG.md` itself.

## Generated files

`CHANGELOG.md`, `docs/protocol.md` and `docs/shed.openrpc.json` are generated: `just generate` writes them (it
is the first step of `just check`), and `just arch` fails a PR whose copies differ from what it writes. After a
merge conflict in one of them, rerun `just generate` instead of merging by hand. The protocol doc's prose is
the `///` docs of the `shed-proto` types; `scripts/changelog.py --release <version>` folds the fragments into
a dated release section.

By contributing you agree your work is dual-licensed under MIT OR Apache-2.0.
