#!/usr/bin/env python3
"""Render CHANGELOG.md's Unreleased section from changelog.d/ (#116), in the current directory.

A fragment is `changelog.d/<kind>-<issue>[-<slug>].md`: one or more markdown bullets. The script replaces
everything between `## [Unreleased]` and the next `## [` with the fragments grouped by kind, in KINDS order,
sorted by issue number. With `--release <version>` it writes them under `## [<version>] - <date>` below an
empty Unreleased section and deletes them. A folder with neither file nor fragments is left alone.
"""

import argparse
import datetime
import pathlib
import re
import sys

KINDS = ["added", "changed", "fixed", "removed", "deprecated", "security"]
NAME = re.compile(rf"^({'|'.join(KINDS)})-(\d+)(-[a-z0-9-]+)?\.md$")
UNRELEASED = "## [Unreleased]\n"


def fragments(folder):
    found = []
    for path in sorted(folder.glob("*")) if folder.is_dir() else []:
        match = NAME.match(path.name)
        if not match:
            sys.exit(f"{path}: not <kind>-<issue>[-<slug>].md, kind one of {', '.join(KINDS)}")
        body = path.read_text().strip()
        if not body.startswith("- "):
            sys.exit(f"{path}: not a markdown bullet list")
        found.append((KINDS.index(match[1]), int(match[2]), path.name, path, body))
    return sorted(found)


def render(found):
    out = ""
    for i, kind in enumerate(KINDS):
        bodies = [body for k, _, _, _, body in found if k == i]
        if bodies:
            out += f"### {kind.capitalize()}\n\n" + "\n".join(bodies) + "\n\n"
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--release", metavar="VERSION")
    args = parser.parse_args()
    changelog, folder = pathlib.Path("CHANGELOG.md"), pathlib.Path("changelog.d")
    found = fragments(folder)
    if not changelog.exists():
        if found:
            sys.exit(f"{folder}: fragments but no {changelog}")
        return
    text = changelog.read_text()
    head, sep, rest = text.partition(UNRELEASED)
    if not sep:
        sys.exit(f"{changelog}: no {UNRELEASED.strip()} section")
    released = rest.find("\n## [")
    tail = rest[released + 1 :] if released >= 0 else ""
    section = render(found)
    if args.release:
        date = datetime.date.today().isoformat()
        section = f"## [{args.release}] - {date}\n\n{section}" if section else ""
        tail, section = section + tail, ""
    changelog.write_text(head + UNRELEASED + "\n" + (section + tail).rstrip("\n") + "\n")
    if args.release:
        for *_, path, _ in found:
            path.unlink()


if __name__ == "__main__":
    main()
