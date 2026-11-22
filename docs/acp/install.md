# Install an agent

```sh
shed agents add claude-acp      # or codex-acp, pi-acp, gemini, …
shed agents                     # id, pinned version, distribution, command
shed agents rm claude-acp
shed agents sessions claude-acp # shed's sessions of this agent: id, last turn (unix s), folder, title
```

`add` reads the ACP registry (`--registry <url|file>`, default
`https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json`) and installs the version it lists now:
this platform's `binary` entry (downloaded with `curl`, checked against its `sha256`, unpacked under
`agents/<id>/<version>/` beside the state file), else the `npx` package (installed once with
`npm install --prefix agents/<id>/<version>` and run as `node <its bin>`, so no `npm exec` stays beside the
adapter), else `uvx <package>`. The entry's `args` and `env` are kept. A failed `npm install`, or a package with
no `bin`, fails `add` and records nothing; an agent recorded earlier as `npx -y …` runs as before until `rm` and
`add` move it to the new form. The result goes to `agents.json` beside the state file. Nothing upgrades an
agent: `rm` it and `add` it again.
