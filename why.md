Decided: git mv the rules folder to .drover and rewrite every old-folder path, docs/architecture included
Because: "update every path that names it"
Because: "No `.herdsman/` path is left anywhere in the tree"
Decided: drop the two ADR notes saying there is no .drover/arch.toml, since the move makes them false
Rejected: leaving the docs/architecture hits — the git grep acceptance check would still print them
Check: each of the nine arch fixtures fails on its own violation with the new --config; the old path exits 0 with nothing to check
Owner: the fleet rules folder moved from .herdsman to .drover; rules and checks are unchanged
