Closes #170

## For humans

Adding an agent that ships as an npm package now installs it once, beside the agent's other files, and runs it directly with node, instead of having npx start it on every launch. Each awake lane of such an agent uses about 120 MB less memory, starts a little faster, and no longer re-checks npm on every start or wake. Agents added the old way keep working; removing one and adding it again moves it to the new way.

## What changed

- `shed agents add` for an `npx` entry runs `npm install --no-audit --no-fund --prefix=agents/<id>/<version> <package>`, reads the installed package's `bin` (a string, or the first entry of the object), and records `node <agents/<id>/<version>/node_modules/<name>/<bin>>` plus the registry's `args`, with `env K=V` first as before. `distribution` stays `npx`.
- `rm` already deletes `agents/<id>/`, so it removes the npm install the same way it removes binary installs.
- A failed `npm install`, or a package with no resolvable bin, fails `add` with `Error::Agents` naming the package (npm's error shows the package in its argument list; a missing bin gives `<package>: no bin to run in its package.json`), and nothing goes into `agents.json`.
- Commands recorded earlier as `npx -y …` are run as stored, with no change to the run path.
- docs/acp.md (the install-order paragraph), the `Agent::distribution` doc line (regenerated docs/shed.openrpc.json) and a changelog fragment.

Decided: the installed package's name comes from the prefix's `package.json` `dependencies`, not from parsing `name@version`, so local tarballs (the test) and scoped names both work.

## Awake RSS, one Claude lane (`scripts/measure-claude-direct.py --runs 1`)

macOS 27.2 arm64, 12 cpus, node v24.22.2, adapter `@agentclientprotocol/claude-agent-acp@0.107.0` (SDK 0.3.323), claude 2.1.323:

| mode | ready | awake RSS |
|---|---|---|
| before: `npx -y` | 1338 ms | 556336 KiB (npm exec 111936 + adapter 132128 + claude 312272) |
| after: `node <bin>` | 1032 ms | 436016 KiB (adapter 131168 + claude 304848) |

Note: the live registry now lists a binary for `claude-acp` (0.108.1), so `shed agents add claude-acp` takes the binary path on this platform. I checked that with the head build (`add` exit 0, `rm` exit 0).

## Cap

`src/ net +14 (budget +25–40)`: one `install_npx` helper replaces the old npx branch. After #173 raised line_cap to 12612 (q2133), `scripts/line-cap.sh` gives 12112 / cap 12612 (exit 0).

## Evidence at head 65c2b76dfda44893b0426a495b11e8d65bff367d (origin/staging 9799fe7 merged in)

- `cargo test -p shed --test acp registry_installs` → exit 0 (fake local npm tarballs, no network: the recorded `node` command, running the installed bin, the npm-failure and no-bin failures with nothing recorded, rm removing `agents/npx-agent`)
- `cargo test --workspace --locked` → exit 101: the only failure is `machine_stats_here_and_on_a_machine_and_an_offline_one_answers_within_5_s` (`uptime_s` 0 on this host). Its #159 why note records it failing on base too. Everything else passes.
- `cargo fmt --all --check` 0 · `cargo clippy --workspace --all-targets --locked -- -D warnings` 0 · `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked` 0 · `scripts/comment-rule.py` 0 · `scripts/line-cap.sh` 0 (12112 / 12612)
- Not run here: `cargo deny` (not installed; no new dependency), and `drover arch-check`'s generated-file step, which needs `just` (not installed). The generate commands were run by hand at 16a2717, and the staging merge touched no generated file.
