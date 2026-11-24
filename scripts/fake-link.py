#!/usr/bin/env python3
"""A fake `ssh` that joins a target to a shed in a temp folder and applies the faults a file names (#308).

Put it first on PATH as `ssh`. `ssh [-o K=V]... [-R remote:local] [--] TARGET CMD...` runs CMD with
HOSTS/TARGET first on PATH (HOSTS is $FAKE_LINK_HOSTS, else the parent of this file's folder), relaying stdin
and stdout through the faults. With `-R remote:local` it relays the unix socket `remote` to `local` until
stdin ends. Each call's arguments but a forward's are appended to `ssh.log` and each byte sent toward a target to
`sent.bytes`, both beside this file.

The faults file ($FAKE_LINK_FAULTS, else `faults` beside this file) is read again on every chunk and every
connect, so a test changes a fault by rewriting it. One fault a line, `TARGET|* FAULT [VALUE]`:

  latency MS   every chunk waits MS milliseconds
  loss PCT     PCT percent of chunks wait a 200 ms retransmit (TCP turns a lost packet into delay)
  stall SECS   nothing moves until SECS after the file was last written
  hang         nothing moves on open links or new ones, and nothing closes
  cut          open links close and new ones fail at once (exit 255)
  gone         the host is gone: open links hang, new ones fail after ConnectTimeout (or VALUE) seconds
  moved OTHER  the address now reaches OTHER: open links hang, new ones reach OTHER
  rekey        the host key changed: new links fail (exit 255), open ones go on
"""

import os
import random
import socket
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
HOSTS = os.environ.get("FAKE_LINK_HOSTS") or os.path.dirname(HERE)
FAULTS = os.environ.get("FAKE_LINK_FAULTS") or os.path.join(HERE, "faults")
POLL = 0.05


def faults(target):
    try:
        with open(FAULTS) as f:
            text, written = f.read(), os.fstat(f.fileno()).st_mtime
    except OSError:
        return {}, 0.0
    found = {}
    for words in map(str.split, text.splitlines()):
        if len(words) >= 2 and words[0] in (target, "*"):
            found[words[1]] = words[2] if len(words) > 2 else ""
    return found, written


def fail(message):
    try:
        sys.stderr.write(message + "\n")
        sys.stderr.flush()
    finally:
        os._exit(255)


def connect(target, options):
    found, _ = faults(target)
    if "cut" in found:
        fail(f"ssh: connect to host {target} port 22: Connection refused")
    if "rekey" in found:
        fail("@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @@@\nHost key verification failed.")
    if "gone" in found:
        time.sleep(float(found["gone"] or options.get("ConnectTimeout", "10")))
        fail(f"ssh: connect to host {target} port 22: Operation timed out")
    return found.get("moved") or target


def gate(target, reached):
    while True:
        found, written = faults(target)
        if "cut" in found:
            fail("Connection to host closed by remote host.")
        held = "hang" in found or "gone" in found or (found.get("moved") or target) != reached
        if "stall" in found and time.time() < written + float(found["stall"] or 0):
            held = True
        if not held:
            delay = float(found.get("latency") or 0) / 1000
            if random.random() * 100 < float(found.get("loss") or 0):
                delay += 0.2
            time.sleep(delay)
            return
        time.sleep(POLL)


def watch(target):
    while "cut" not in faults(target)[0]:
        time.sleep(POLL)
    fail("Connection to host closed by remote host.")


def kept(data):
    with open(os.path.join(HERE, "sent.bytes"), "ab") as log:
        log.write(data)


def pump(read, write, target, reached, keep):
    try:
        while data := read():
            gate(target, reached)
            if keep:
                kept(data)
            write(data)
    except OSError:
        pass


def write_all(fd):
    def write(data):
        while data:
            data = data[os.write(fd, data):]

    return write


def pipe(a, b, target, reached, keep):
    pump(lambda: a.recv(65536), b.sendall, target, reached, keep)
    try:
        b.shutdown(socket.SHUT_WR)
    except OSError:
        pass


def forward(spec, target, reached):
    remote, _, local = spec.partition(":")
    server = socket.socket(socket.AF_UNIX)
    server.bind(remote)
    server.listen()

    def serve():
        while True:
            conn, _ = server.accept()
            up = socket.socket(socket.AF_UNIX)
            up.connect(local)
            for a, b, keep in ((conn, up, False), (up, conn, True)):
                threading.Thread(target=pipe, args=(a, b, target, reached, keep), daemon=True).start()

    threading.Thread(target=serve, daemon=True).start()
    while os.read(0, 65536):
        pass
    os._exit(0)


def main():
    args = sys.argv[1:]
    line = " ".join(args) + "\n"
    options, spec = {}, None
    while args[:1] in (["-o"], ["-R"]):
        flag, value, args = args[0], args[1], args[2:]
        if flag == "-o":
            key, _, options[key] = value.partition("=")
        else:
            spec = value
    if args[:1] == ["--"]:
        args = args[1:]
    if not spec:
        with open(os.path.join(HERE, "ssh.log"), "a") as log:
            log.write(line)
    target, command = args[0], args[1:]
    reached = connect(target, options)
    threading.Thread(target=watch, args=(target,), daemon=True).start()
    if spec:
        forward(spec, target, reached)
    path = os.path.join(HOSTS, reached) + os.pathsep + os.environ.get("PATH", "")
    child = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, env={**os.environ, "PATH": path})

    def up():
        pump(lambda: os.read(0, 65536), write_all(child.stdin.fileno()), target, reached, True)
        child.stdin.close()

    threading.Thread(target=up, daemon=True).start()
    pump(lambda: os.read(child.stdout.fileno(), 65536), write_all(1), target, reached, False)
    os._exit(child.wait())


main()
