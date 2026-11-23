# CI

`.github/workflows/ci.yml` is one call of gatekeeper's `gates.yml`, pinned by commit (`./.gatekeeper/run pin
<version>` moves it). It runs the same gates as `./.gatekeeper/run check` (`just check`): the core and rust packs
and shed's own in `.gatekeeper/gates/`, one job per leg (`core`, `rust`, `local`, `macos`, `bench-acp`,
`bench-drain`, `bench-fanout`), and `gates / done`, the one check branch protection requires. A new push to a PR
cancels that PR's older run; pushes to `staging` and `main` are never cancelled.

## The macOS runner

Hosted macOS jobs share an account-wide queue and can wait hours, so the `macos` leg runs on
a self-hosted runner on the mini when the repository variable `CI_RUNNER` is set. It holds the runner's
labels as a JSON list:

```sh
gh variable set CI_RUNNER --body '["self-hosted","macOS","shed"]'
```

With the variable unset (`gh variable delete CI_RUNNER`), the leg runs on `macos-latest`.

shed is public, so only same-repo code reaches the runner: pushes to `staging`/`main` and PRs whose
head branch is in this repo. Fork PRs always get `macos-latest`. `local/self-hosted` fails any workflow
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

## The multi-host fault gate and the nightly soak (#308)

`scripts/fake-link.py` stands in for `ssh`: it joins a target to a shed in a temp folder and applies the faults
its `faults` file names (latency, loss, stall, hang, cut, gone, moved, rekey; the script's docstring has the
format). shed's machine tests (`crates/shed/tests/machines/`) all run over it, and `faults.rs` gives each fault
its own test with a time bound. `local/fake-link` runs them on any PR that touches the link, machine, exec/file,
stats, account or token-proxy code. Each release carries the script as `shed-fake-link.py`, so drover's tests can
put it on PATH as `ssh` and set `FAKE_LINK_HOSTS` and `FAKE_LINK_FAULTS`.

`.github/workflows/soak.yml` runs `scripts/soak.py` nightly on the self-hosted runner: this machine and each
`label=ssh-target` in the repository variable `SOAK_MACHINES`, timed against `ci/soak.toml`. The run's summary
and its `soak` artifact hold the numbers; each missed target files one issue that night.
