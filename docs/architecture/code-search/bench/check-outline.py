#!/usr/bin/env python3
"""Compares the prototype's Rust outline with a bracket-matching lexer, on every tracked crates/**.rs.

    python3 -I docs/architecture/code-search/bench/check-outline.py "$TMPDIR/proto"

For each symbol it finds the true end (the brace that closes the item's first `{`, or its `;`) with
strings, chars and comments blanked, and prints the symbols whose outline end differs.
"""

import json, re, subprocess, sys
proto = sys.argv[1]
files = subprocess.run(["git", "ls-files", "crates/*.rs"], capture_output=True, text=True).stdout.split()

def code_mask(src):
    """A copy of src with strings, chars and comments blanked (newlines kept)."""
    out, i, n = [], 0, len(src)
    def blank(to):
        nonlocal i
        out.append("".join(c if c == "\n" else " " for c in src[i:to]))
        i = to
    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i); blank(n if j < 0 else j)
        elif src.startswith("/*", i):
            j = src.find("*/", i + 2); blank(n if j < 0 else j + 2)
        elif c == "r" and re.match(r'r#*"', src[i:]) and (i == 0 or not (src[i-1].isalnum() or src[i-1] == "_")):
            hashes = len(re.match(r'r(#*)"', src[i:]).group(1))
            j = src.find('"' + "#" * hashes, i + 2 + hashes); blank(n if j < 0 else j + 1 + hashes)
        elif c == '"':
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            blank(min(n, j + 1))
        elif c == "'":
            m = re.match(r"'(\\.[^']*|[^\\'])'", src[i:])
            if m:
                blank(i + m.end())
            else:
                out.append(c); i += 1
        else:
            out.append(c); i += 1
    return "".join(out)

total = wrong = 0
for f in files:
    src = open(f).read()
    doc = json.loads(subprocess.run([proto, "outline", "--json", f], capture_output=True, text=True).stdout)
    code = code_mask(src)
    starts = [0]
    for m in re.finditer("\n", code):
        starts.append(m.end())
    raw = src.split("\n")
    for s in doc["symbols"]:
        total += 1
        head = next(k for k in range(s["start"] - 1, s["end"]) if not raw[k].strip().startswith(("//", "#[", "#![")) and raw[k].strip())
        i, depth, opened, end = starts[head], 0, False, None
        while i < len(code):
            c = code[i]
            if c in "{([":
                depth += 1; opened = opened or c == "{"
            elif c in "})]":
                depth -= 1
                if depth == 0 and c == "}" and opened:
                    end = code.count("\n", 0, i) + 1; break
            elif c == ";" and depth == 0 and not opened:
                end = code.count("\n", 0, i) + 1; break
            i += 1
        if end != s["end"]:
            wrong += 1
            print(f"{f}: {s['kind']} {s['name']} outline [{s['start']}-{s['end']}] lexer end {end}")
print(f"{len(files)} files, {total} symbols, {wrong} ends differ from a bracket-matching lexer")
