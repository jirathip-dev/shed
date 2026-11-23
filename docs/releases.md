# Releases (#160)

shed ships as CI's signed release builds, the way drover does (drover's `docs/releases.md`), so no host has to
build it from source with `cargo install`.

## The release workflow

`.github/workflows/release.yml` runs after the `ci` workflow ends green on a push to main or staging (`workflow_run`).
GitHub runs a `workflow_run` workflow from the default branch's copy, so it starts once the file is on main. It
builds `aarch64-apple-darwin` (macOS runner) and `x86_64-unknown-linux-gnu` (the oldest hosted Ubuntu, for a low
glibc floor), then publishes one GitHub release:

| | main | staging |
|---|---|---|
| tag | `build-<sha7>` | `build-staging-<sha7>` |
| kind | release, kept for good | prerelease; the staging run deletes those over 14 days old, tag included |
| assets | `shed-aarch64-apple-darwin`, `shed-x86_64-unknown-linux-gnu`, `shed-fake-link.py` (#308), `SHA256SUMS` | the same |

`shed` is the only binary: the server is `shed serve`. `shed --version` prints `<version> (<sha7>)`, the commit it was
built from; each build job checks that it names the sha the release is tagged with, so an installer can check it too.

The macOS binary is signed with the owner's Developer ID Application certificate (hardened runtime, secure timestamp,
identifier `shed`) and notarized with an App Store Connect API key by `scripts/release-sign.sh` (drover's script with
the identifier changed). It then checks the binary with `codesign -dv`, `codesign -v -R=notarized --check-notarization`
and `spctl -a -vv -t install`. A bare binary cannot carry a stapled ticket, so the notarization is recorded with Apple
and checked online. Only the signed binary is published.

A failed build, signing or publish step ends its job green with an error annotation and publishes nothing, so the
workflow never turns the branch red. The failure shows up as a missing release. Re-run the workflow run to retry.

### Secrets

The owner sets these in the repo's `release` environment (branches main and staging). They are the same certificate
and key as drover's, and only the macOS job reads them:

| Name | What |
|---|---|
| `MACOS_CERT_P12` | the Developer ID Application certificate and key, a base64 `.p12` |
| `MACOS_CERT_PASSWORD` | that file's password |
| `ASC_API_KEY_P8` | the App Store Connect API key, a base64 `.p8` |
| `ASC_API_KEY_ID` | its key id |
| `ASC_API_ISSUER_ID` | its issuer id |

## Client releases (#267)

Drover, cooee and any other client depend on shed's client crates at a tag instead of hand-rolling a client:

```toml
shed-client = { git = "https://github.com/jirathip-dev/shed", tag = "client-v4.0" }
```

The release workflow's `client-tag` job runs after `ci` ends green on a push to main. `scripts/client-tag.sh`
finds the newest `client-v*` tag reachable from the commit and, when `crates/shed-proto`, `crates/shed-client`
or `crates/shed-catalog` changed since then (or there is no tag yet), pushes `client-v<PROTOCOL>.<n>` on it:
`<PROTOCOL>` is `shed_proto::PROTOCOL` at that commit and `<n>` counts from 0 within each protocol. A run with no
client change, or a re-run on a tagged commit, pushes nothing. The tag is git only, with no release assets.

Starting the workflow by hand (`gh workflow run release.yml -R jirathip-dev/shed`) is a dry run: it builds and
publishes nothing and prints the tag it would push, or `no client change since <tag>`. Locally,
`scripts/client-tag.sh [<commit>]` prints the same; `--push` pushes it.

## A manual install

Download the asset for the machine from the release of the sha you want, check it, and put it in place:

```sh
t=build-<sha7>; a=shed-aarch64-apple-darwin   # or shed-x86_64-unknown-linux-gnu
gh release download "$t" -R jirathip-dev/shed -p "$a" -p SHA256SUMS
grep " $a\$" SHA256SUMS | shasum -a 256 -c -
chmod 755 "$a" && "./$a" --version && mv "$a" ~/.local/bin/shed
```

This manual install is the only one: once the binary is in place, `shed install` adds the job that keeps it on
main's latest release, and handing serve over keeps the terminals running ([self-update.md](self-update.md)).
