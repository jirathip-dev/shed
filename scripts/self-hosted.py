#!/usr/bin/env python3
"""Fork code never reaches a self-hosted runner (#93), in every workflow under DIR (default .github/workflows).

shed is public: a job that names `self-hosted` or the `vars.CI_RUNNER` labels must gate that runner on
`github.event.pull_request.head.repo.full_name == github.repository`, in its `runs-on` or its job `if:`.
A job's steps are not read.
"""

import pathlib
import re
import sys

GUARD = "github.event.pull_request.head.repo.full_name == github.repository"
SELF_HOSTED = re.compile(r"self-hosted|vars\.CI_RUNNER")
JOB = re.compile(r"^  ([\w-]+):\s*$")
KEY = re.compile(r"^    ([\w-]+):")


def unguarded(path):
    jobs, name = {}, None
    in_jobs = False
    for line in path.read_text().splitlines():
        if re.match(r"^\S", line):
            in_jobs, name = line.startswith("jobs:"), None
            continue
        if in_jobs and JOB.match(line):
            name = JOB.match(line).group(1)
            jobs[name] = []
        elif in_jobs and name and not re.match(r"^    steps:", line):
            jobs[name].append(line)
        elif in_jobs:
            name = None
    for name, lines in jobs.items():
        if not SELF_HOSTED.search("\n".join(lines)):
            continue
        guarded, key = False, None
        for line in lines:
            m = KEY.match(line)
            key = m.group(1) if m else key
            if key in ("runs-on", "if") and GUARD in line:
                guarded = True
        if not guarded:
            yield f"{path}: job `{name}` uses a self-hosted runner without `{GUARD}`"


root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ".github/workflows")
bad = [msg for wf in sorted(root.glob("*.y*ml")) for msg in unguarded(wf)]
for msg in bad:
    print(msg)
if bad:
    sys.exit(1)
print("every self-hosted job is gated to same-repo code")
