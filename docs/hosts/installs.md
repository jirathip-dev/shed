# Installs (#345)

shed moves releases of two binaries only, on the main host: its own, and the binary of the tool that calls it
(drover's at drover's path). It installs no toolchain, agent CLI or cache: a shed machine is only a place to
run an agent.

## Which binary is the caller's

A client names its own tool at `hello`, as `tool: {name, exe}`: the name its release assets start with, and the
absolute path of its live binary, whose last part must be that name (`drover` at `/home/user/.cargo/bin/drover`).
`shed_client::Conn::connect_as` and `Shed::tool` send it. The `install` methods on that connection accept
`shed` with shed's own binary (the running server's) and that tool with that path; any other name or path is
`NOT_OWN_BINARY` and nothing changes. A client that names no tool moves shed's binary only.

## The methods

| method | does |
|---|---|
| `install.find` | reads `<releases>/<tag>/SHA256SUMS`; none is `NO_RELEASE` |
| `install.ls` | the `--version` line of `<dest>`, `<dest>.new` and `<dest>.prev` |
| `install.stage` | downloads `<tool>-<arch>-<os>` to `<dest>.new.part`, checks its sha256, makes it 755, checks that its `--version` contains `build`, then renames it to `<dest>.new`; a failed check removes the part file and is `UNSTAGED` naming it (`asset`, `download`, `checksum`, `version`). Already staged: nothing is fetched |
| `install.swap` | once `<dest>.new --version` answers with `build`: hard-links the live binary to `<dest>.prev` and renames `<dest>.new` over it; else `UNSTAGED` and nothing moves. Live already `build`: nothing |
| `install.back` | once `<dest>.prev --version` answers: renames it over the live binary; else `UNSTAGED` |

The old binary stays live until the new one has answered, and stays as `<dest>.prev` after a swap. When and
which build to install, what runs after a swap and whether to go back are the caller's decisions.

```console
$ shed install ls
live    0.1.0 (1a2b3c4)
staged  -
prev    0.1.0 (4254d78)
```

[Self-update](../self-update.md) runs the same `find`, `stage` and `swap` on shed's own binary, in process.
