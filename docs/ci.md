# CI

`.github/workflows/ci.yml` runs every gate on Ubuntu (`msrv`, `rules`, the benches) and the tests on
Ubuntu and macOS (`stable`). A new push to a PR cancels that PR's older run; pushes to `staging` and
`main` are never cancelled.

## The macOS runner

Hosted macOS jobs share an account-wide queue and can wait hours, so the macOS leg of `stable` runs on
a self-hosted runner on the mini when the repository variable `CI_RUNNER` is set. It holds the runner's
labels as a JSON list:

```sh
gh variable set CI_RUNNER --body '["self-hosted","macOS","shed"]'
```

With the variable unset (`gh variable delete CI_RUNNER`), the leg runs on `macos-latest`.

shed is public, so only same-repo code reaches the runner: pushes to `staging`/`main` and PRs whose
head branch is in this repo. Fork PRs always get `macos-latest`. `just self-hosted` fails any workflow
job that names a self-hosted runner without that check.

### Registering it on the mini

Get a registration token from Settings → Actions → Runners → New self-hosted runner (macOS, ARM64),
download the runner there into `~/actions-runner`, then:

```sh
cd ~/actions-runner
./config.sh --url https://github.com/jirathip-dev/shed --token <TOKEN> \
  --name shed-mini --labels shed --work _work --unattended
./svc.sh install && ./svc.sh start
```

- **Labels:** `self-hosted`, `macOS` and `ARM64` are added by the runner; `--labels shed` adds `shed`.
- **Work folder:** `~/actions-runner/_work` (checkouts and build output; safe to delete between jobs).
- **User:** `svc.sh` installs a launchd user agent, so the runner runs as the normal login user, not
  root. It needs Rust via rustup (the workflow installs the toolchain) and no secrets beyond the
  per-job `GITHUB_TOKEN` GitHub hands it.

`./svc.sh stop && ./svc.sh uninstall && ./config.sh remove --token <TOKEN>` takes it off again; unset
`CI_RUNNER` first so jobs fall back to `macos-latest`.
