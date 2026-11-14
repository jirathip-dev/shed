# Contributing

## Run the gates locally

Install [just](https://github.com/casey/just) and [cargo-deny](https://github.com/EmbarkStudios/cargo-deny), then:

```sh
just check
```

It runs every CI gate: `fmt`, `clippy` (pedantic, at most 10 allows, each with a reason), `test`, `doc`,
`deny`, `cap` (≤ 6,200 non-test lines across `crates/*/src`), `deps` (`shed-core` has no workspace deps)
and `hygiene` (no setup-specific names, see `ci/denylist.txt`). Each one also runs alone, e.g. `just clippy`.
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
