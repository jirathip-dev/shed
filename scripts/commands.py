#!/usr/bin/env python3
"""Render README.md's command table from docs/commands/*.md (#250), in the current directory.

A row is `docs/commands/<command>.md`: its first paragraph is the Command cell, the rest, joined by single
spaces, the What-it-does cell; a `|` in either is escaped. The script replaces everything between
`<!-- commands -->` and `<!-- /commands -->` with the table, rows in file-name order, and leaves the rest of
the README alone. A folder with neither README.md nor docs/commands/ is left alone.
"""

import pathlib
import sys

START, END = "<!-- commands -->\n", "<!-- /commands -->\n"
HEAD = "| Command | What it does |\n|---|---|\n"


def cell(lines):
    return " ".join(line.strip() for line in lines if line.strip()).replace("|", "\\|")


def row(path):
    command, _, what = path.read_text().strip().partition("\n\n")
    if not what.strip():
        sys.exit(f"{path}: no What-it-does paragraph after the Command line")
    return f"| {cell(command.splitlines())} | {cell(what.splitlines())} |\n"


def main():
    readme, folder = pathlib.Path("README.md"), pathlib.Path("docs/commands")
    if not readme.exists() and not folder.is_dir():
        return
    text = readme.read_text()
    head, start, rest = text.partition(START)
    _, end, tail = rest.partition(END)
    if not start or not end:
        sys.exit(f"{readme}: no {START.strip()} … {END.strip()} markers")
    rows = "".join(row(path) for path in sorted(folder.glob("*.md"), key=lambda p: p.stem))
    readme.write_text(head + START + HEAD + rows + END + tail)


if __name__ == "__main__":
    main()
