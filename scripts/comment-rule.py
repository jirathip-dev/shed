#!/usr/bin/env python3
"""The comment rule (#65), on every tracked .rs file outside ci/fixtures (or every .rs under ROOT).

No `//` or `/* */` comments, except `// SAFETY:` and `// TODO(#N)`. `///` only on public API: a bare
`pub` item, or an item of a `pub enum` or `pub trait`, in a public module of a library crate's src/.
`//!` only at the top of such a module's file, or of a crate root (missing_docs wants its crate doc).
"""

import os
import re
import subprocess
import sys

ALLOWED = re.compile(r"// (SAFETY: |TODO\(#\d+\))")
IMPLICIT = re.compile(r"pub (enum|trait) ")
CONTAINER = re.compile(r"(pub (mod|struct|union|enum|trait) |impl\b)")
CRATE_ROOT = re.compile(r"crates/[^/]+/(src/(lib|main)|src/bin/[^/]+|(tests|benches|examples)/[^/]+(/main)?|build)\.rs")
KEYWORD = re.compile(r"(pub\b|fn |mod |enum |struct |trait |union |impl\b|const |static |type |let |match |if |for |while |loop\b|unsafe |async |macro_rules!)")


def lex(src):
    """Comments as (line, col, text), and the set of lines that start inside a literal or block comment."""
    comments, inside, i, line, n = [], set(), 0, 1, len(src)

    def advance(to):
        nonlocal i, line
        for ch in src[i:to]:
            if ch == "\n":
                line += 1
                inside.add(line)
        i = to

    while i < n:
        c = src[i]
        if src.startswith("//", i):
            end = src.find("\n", i)
            end = n if end < 0 else end
            col = i - src.rfind("\n", 0, i) - 1
            comments.append((line, col, src[i:end]))
            i = end
        elif src.startswith("/*", i):
            start, col, depth, j = line, i - src.rfind("\n", 0, i) - 1, 0, i
            while j < n:
                if src.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif src.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                    if depth == 0:
                        break
                else:
                    j += 1
            comments.append((start, col, src[i:j]))
            advance(j)
        elif m := re.match(r'(b|c)?r(#*)"', src[i:]) if c in "bcr" and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")) else None:
            close = '"' + m.group(2)
            end = src.find(close, i + m.end())
            advance(n if end < 0 else end + len(close))
        elif c == '"':
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            advance(j + 1)
        elif c == "'" and (m := re.match(r"'(\\.|\\u\{[0-9a-fA-F]+\}|\\x..|[^\\'])'", src[i:])):
            i += m.end()
        elif c == "\n":
            line += 1
            i += 1
        else:
            i += 1
    return comments, inside


def public_files(root, files):
    """The .rs files that are public modules of a library crate: lib.rs, and each `pub mod` chain from it."""
    public = set()
    for f in files:
        parts = f.split("/")
        if len(parts) < 4 or parts[0] != "crates" or parts[2] != "src":
            continue
        if not os.path.exists(os.path.join(root, *parts[:3], "lib.rs")):
            continue
        mods = parts[3:]
        mods[-1] = mods[-1][:-3]
        if mods[-1] == "mod":
            mods.pop()
        if mods == ["lib"]:
            public.add(f)
            continue
        ok, parent = True, os.path.join(root, *parts[:3], "lib.rs")
        for k, name in enumerate(mods):
            try:
                with open(parent, encoding="utf-8") as fh:
                    text = fh.read()
            except OSError:
                ok = False
                break
            if not re.search(rf"^pub mod {re.escape(name)};", text, re.M):
                ok = False
                break
            base = os.path.join(root, *parts[:3], *mods[: k + 1])
            parent = base + ".rs" if os.path.exists(base + ".rs") else os.path.join(base, "mod.rs")
        if ok:
            public.add(f)
    return public


def indent(s):
    return len(s) - len(s.lstrip())


def item_start(lines, inside, k):
    """Walk back from a block-opening line (`) -> T {`, `{`, `where`) to the line that starts its item."""
    lvl = indent(lines[k])
    while k > 0:
        t = lines[k].strip()
        if not (t.startswith((")", "{", "where", ">")) or t == ""):
            break
        k -= 1
        while k > 0 and (k + 1 in inside or not lines[k].strip() or indent(lines[k]) > lvl):
            k -= 1
    return k


def ancestors(lines, inside, k):
    """The item-start lines of every block enclosing line k, innermost first."""
    out, lvl = [], indent(lines[k])
    while lvl > 0 and k > 0:
        k -= 1
        if k + 1 in inside or not lines[k].strip() or indent(lines[k]) >= lvl:
            continue
        k = item_start(lines, inside, k)
        lvl = indent(lines[k])
        out.append(lines[k].strip())
    return out


def is_public_item(lines, inside, k):
    chain = [lines[k].strip()] + ancestors(lines, inside, k)
    for i, item in enumerate(chain):
        if i > 0 and not (CONTAINER.match(item) or not KEYWORD.match(item)):
            return False
        parent = chain[i + 1] if i + 1 < len(chain) else ""
        if not (item.startswith("pub ") or item.startswith("impl") or IMPLICIT.match(parent) or (parent and not KEYWORD.match(parent))):
            return False
    return True


def doc_target(lines, inside, k):
    """The line the doc comment on line k (0-based) documents: past further docs and attributes."""
    lvl = indent(lines[k])
    for j in range(k + 1, len(lines)):
        t = lines[j].strip()
        if j + 1 in inside or not t or indent(lines[j]) > lvl or t.startswith(("///", "#", ")", "]")):
            continue
        return j
    return None


def violations(root, f, public):
    with open(os.path.join(root, f), encoding="utf-8") as fh:
        src = fh.read()
    comments, inside = lex(src)
    lines = src.split("\n")
    code_before = {n for n, l in enumerate(lines, 1) if l.strip() and not l.strip().startswith("//")}
    out = []
    for line, col, text in comments:
        if ALLOWED.match(text):
            continue
        ok = False
        if text.startswith("///") and not text.startswith("////") and f in public:
            j = doc_target(lines, inside, line - 1)
            ok = j is not None and is_public_item(lines, inside, j)
        elif text.startswith("//!") and (f in public or CRATE_ROOT.fullmatch(f)):
            ok = not any(n < line for n in code_before)
        if not ok:
            out.append((line, col, text))
    return out


def main():
    root = sys.argv[1] if len(sys.argv) > 1 else "."
    if root == ".":
        files = subprocess.run(
            ["git", "ls-files", "*.rs", ":!ci/fixtures"], capture_output=True, text=True, check=True
        ).stdout.split()
    else:
        files = sorted(
            os.path.relpath(os.path.join(d, n), root)
            for d, _, ns in os.walk(root)
            for n in ns
            if n.endswith(".rs")
        )
    public = public_files(root, files)
    bad = [(f, *v) for f in files for v in violations(root, f, public)]
    for f, line, col, text in bad:
        print(f"{os.path.normpath(os.path.join(root, f))}:{line}:{col + 1}: {text.splitlines()[0]}")
    if bad:
        print("comments break the rule (.herdsman/rules.md): no // or /* */ but // SAFETY: and // TODO(#N); /// only on public API")
        return 1
    print("comments follow the rule")
    return 0


if __name__ == "__main__":
    sys.exit(main())
