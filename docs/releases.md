# Releases (#160)

shed ships as CI's signed release builds, the way drover does (drover's `docs/releases.md`), so no host has to
build it from source with `cargo install`.

## The release workflow

`.github/workflows/release.yml` runs after the `ci` workflow ends green on a push to main or staging (`workflow_run`),
a re-run of `ci` included. GitHub runs a `workflow_run` workflow from the default branch's copy, which is staging's. It
builds `aarch64-apple-darwin` (macOS runner) and `x86_64-unknown-linux-gnu` (the oldest hosted Ubuntu, for a low
glibc floor), then publishes one GitHub release:

| | main | staging |
|---|---|---|
| tag | `build-<sha7>` | `build-staging-<sha7>` |
| kind | release, kept for good | prerelease; the staging run deletes those over 14 days old, tag included |
| assets | `shed-aarch64-apple-darwin`, `shed-x86_64-unknown-linux-gnu`, `shed-fake-link.py` (#308), `SHA256SUMS` | the same |

`shed` is the only binary: the server is `shed serve`. `shed --version` prints `<version> (<sha7>)`, the commit it was
built from; each build job checks that it names the sha the release is tagged with, so an installer can check it too.

### Finding a main release run (#356)

GitHub lists every `workflow_run` run under the default branch's head branch and sha (staging), whichever branch it
builds, so `gh run list -w release -b main` is always empty. The run title names the built branch and sha
(`release main <sha>`), and so do the release's notes (`shed main @ <sha>`). #356 was filed because main's release
runs looked missing. They were not: main 7540e74's first green `ci` started a release run that failed on #338's missing
`scripts/fake-link.py`, and the `ci` re-run started a release run with the fixed workflow, which published
`build-7540e74`.

### Main behind staging (#338)

A `workflow_run` workflow runs the default branch's (staging's) `release.yml` on a checkout of the built sha, so a
release of main runs a newer `release.yml` than main's own. It must handle main being behind staging: each file it
copies or packs that only newer commits have is skipped when absent, with a `::notice::` that names it. Today that is
`scripts/fake-link.py`, so a release of a sha without it ships no `shed-fake-link.py`. The binaries, `SHA256SUMS` and
the scripts every release has used (`scripts/release-sign.sh`, `scripts/client-tag.sh`) stay required: without one
nothing is published, as before. A new file the workflow ships gets the same skip until main has it.

The macOS binary is signed with the owner's Developer ID Application certificate (hardened runtime, secure timestamp,
identifier `shed`) and notarized with an App Store Connect API key by `scripts/release-sign.sh` (drover's script with
the identifier changed). It then checks the binary with `codesign -dv`, `codesign -v -R=notarized --check-notarization`
and `spctl -a -vv -t install`. A bare binary cannot carry a stapled ticket, so the notarization is recorded with Apple
and checked online. Only the signed binary is published.

A failed build, signing or publish step ends its job green with an error annotation and publishes nothing, so the
workflow never turns the branch red. The failure shows up as a missing release. To retry, re-run the `ci` run, which
starts a release run with today's workflow, or publish by hand. Re-running the failed release run itself replays the
workflow file as it was then.

### Publishing a main sha by hand (#356)

```sh
gh workflow run release.yml -R jirathip-dev/shed -f sha=<main sha> -f publish=true
```

This builds that sha, publishes its `build-<sha7>` release and pushes its client tag, exactly as a green `ci` on main
would have done. The sha must be a commit on main, and `publish=true` needs a sha. Otherwise the run fails before
building anything. A release that already exists is not replaced: the publish step reports an error. Without
`publish=true` the run is a dry run (see below).

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
shed-secrets = { git = "https://github.com/jirathip-dev/shed", tag = "client-v4.0" }   # a program's own secrets (docs/hosts/secrets.md)
```

The release workflow's `client-tag` job runs after `ci` ends green on a push to main. `scripts/client-tag.sh`
finds the newest `client-v*` tag reachable from the commit and, when `crates/shed-proto`, `crates/shed-client`,
`crates/shed-catalog` or `crates/shed-secrets` changed since then (or there is no tag yet), pushes `client-v<PROTOCOL>.<n>` on it:
`<PROTOCOL>` is `shed_proto::PROTOCOL` at that commit and `<n>` counts from 0 within each protocol. A run with no
client change, or a re-run on a tagged commit, pushes nothing. The tag is git only, with no release assets.

Starting the workflow by hand without `publish=true` (`gh workflow run release.yml -R jirathip-dev/shed`, optionally
with `-f sha=<main sha>`) is a dry run: it builds and publishes nothing and prints the tag it would push for the
dispatched ref (or that sha), or `no client change since <tag>`. Locally,
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
