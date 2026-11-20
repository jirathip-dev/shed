"""What a guard costs a call: one process for each call, against one process that answers a line for each.

The guard here is a Python loop that parses each call and answers with its id, as a real one would.
`python3 -I guard-trip.py`
"""

import json
import statistics
import subprocess
import sys
import time

LOOP = "import sys, json\nfor line in sys.stdin:\n    call = json.loads(line)\n    print(json.dumps({'id': call['id'], 'allow': call['method'] != 'kill'}), flush=True)\n"
ONCE = "import sys, json\ncall = json.loads(sys.stdin.readline())\nsys.exit(0 if call['method'] != 'kill' else 3)\n"


def call(n):
    params = {"name": "impl-190", "text": "Read .brief.md fully, then start."}
    caller = {"terminal": "orchestrator-7", "cwd": "/home/user/work/shed/issue-190"}
    return json.dumps({"id": n, "method": "send", "params": params, "machine": None, "caller": caller}) + "\n"


def report(name, took):
    took.sort()
    p99 = took[int((len(took) - 1) * 0.99)]
    print(f"{name:<34} median {statistics.median(took) * 1e3:8.3f} ms   p99 {p99 * 1e3:8.3f} ms   ({len(took)} calls)")


def per_call(n):
    took = []
    for i in range(n):
        start = time.perf_counter()
        done = subprocess.run([sys.executable, "-I", "-c", ONCE], input=call(i).encode(), capture_output=True)
        took.append(time.perf_counter() - start)
        assert done.returncode == 0
    report("a process for each call", took)


def per_run(n):
    guard = subprocess.Popen([sys.executable, "-I", "-c", LOOP], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    took = []
    for i in range(n):
        start = time.perf_counter()
        guard.stdin.write(call(i).encode())
        guard.stdin.flush()
        answer = json.loads(guard.stdout.readline())
        took.append(time.perf_counter() - start)
        assert answer == {"id": i, "allow": True}
    print(f"{'one process for the run: its start':<34} {'':7}{took[0] * 1e3:8.3f} ms   (with its first answer)")
    report("one process for the run: a call", took[1:])
    guard.stdin.close()
    guard.wait()


per_call(100)
per_run(2000)
