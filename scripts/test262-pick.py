#!/usr/bin/env python3
"""Vendors the test262 files shed's script subset runs (#214): `scripts/test262-pick.py <test262 checkout>`.

Reads test/language/ of the checkout, and writes crates/shed-script/tests/test262/: `files/` with the
picked files, `run.txt` (files that must end normally, or with their `negative:` error), `out.txt`
(files that must fail with the SyntaxError naming a left-out construct, at most OUT_PER_TAG a tag),
`skipped.txt` (every other file, by its first tag), the checkout's LICENSE and README.md with its commit.
A file is skipped by its flags, includes, feature tags, or a construct or built-in this slice lacks;
the tests are the harness's (crates/shed-script/tests/test262.rs), never edited here.
"""

import collections
import os
import re
import shutil
import subprocess
import sys

OUT_PER_TAG = 10
FEATURES = {
    "arrow-function", "computed-property-names", "const", "default-parameters", "destructuring-binding",
    "let", "object-rest", "object-spread", "optional-catch-binding", "optional-chaining", "template",
    "coalesce-expression", "exponentiation", "logical-assignment-operators", "numeric-separator-literal",
    "json-superset", "u180e", "rest-parameters", "string-trimming",
}
OUT_FEATURES = {"class": "class", "generators": "generator", "regexp-named-groups": "regex"}
FLAGS = {"onlyStrict", "generated"}
INCLUDES = {"assert.js", "sta.js"}
REFUSED = [
    ("syntax:var", r"\bvar\b", "var"),
    ("syntax:this", r"\bthis\b", "this"),
    ("syntax:new", r"\bnew\b(?!\s+Error\s*\()", "new"),
    ("syntax:class", r"\bclass\b", "class"),
    ("syntax:generator", r"function\s*\*|\byield\b", "generator"),
    ("syntax:switch", r"\bswitch\s*\(", "switch"),
    ("syntax:do", r"\bdo\b", "do"),
    ("syntax:for-in", r"\bfor\s*\((?:\s*(?:let|const)\s+)?\w+\s+in\b", "for … in"),
    ("syntax:finally", r"\bfinally\b", "finally"),
    ("syntax:with", r"\bwith\s*\(", "with"),
    ("syntax:delete", r"\bdelete\b", "delete"),
    ("syntax:void", r"\bvoid\b", "void"),
    ("syntax:in", r"[\w)\]'\"]\s+in\s+[\w(]", "in operator"),
    ("syntax:instanceof", r"\binstanceof\b", "instanceof"),
    ("syntax:getter", r"\bget\s+(?:\w+\s*\(|\[)", "getter"),
    ("syntax:setter", r"\bset\s+(?:\w+\s*\(|\[)", "setter"),
    ("syntax:label", r"(?m)^\s*[A-Za-z_$][\w$]*\s*:\s*(?:for|while|do)\b", "label"),
    ("syntax:label", r"(?m)^\s*\{\s*[A-Za-z_$][\w$]*\s*:\s*\d+\s*$", "label"),
    ("syntax:comma", r"(?m)^\s*0\s*,\s*[\[{]", "comma"),
    ("syntax:regex", r"(?:^|[(,=:\[!&|?{};])\s*/[^/*\n]", "regex"),
    ("syntax:debugger", r"\bdebugger\b", "debugger"),
    ("syntax:escape", r"\\u", "'\\\\'"),
    ("syntax:bitwise", r"~", "bitwise"),
    ("syntax:await", r"\bawait\b", "await"),
]
DIFFERS = [
    ("differs: no temporal dead zone", r"before-initiali[sz]ation|tdz"),
    ("differs: functions have no .name or .length", r"fn-name|expected-argument-count|function-name"),
    ("differs: a script is a function body, not global code", r"^language/global-code/|statements/return/S12\.9_A1_T"),
    ("differs: ?? beside && or || is not refused", r"coalesce/cannot-chain"),
    ("differs: \"use strict\" and __proto__ are not special", r"strict-body|NSPL-with-USD|__proto__|use-strict-with-non-simple-param"),
    ("differs: no implicit conversion", r"unary-minus/11\.4\.7-4-1|unary-plus/11\.4\.6-2-1|asi/S7\.9_A10_T[135]"),
    ("differs: a key is a string or a number", r"optional-chain-expression-optional-expression"),
    ("differs: a name is declared once in a block", r"recursive-array-and-object-patterns"),
    ("syntax:tagged-template", r"tagged-template/"),
    ("harness:eval", r"^language/eval-code/"),
]
KNOWN_OUT = [
    (r"init-hole", "array hole"),
    (r"optional-chain-prod-expression|statementList/.*arrow-function-boolean-literal", "comma"),
    (r"statementList/.*with-labels|statementList/block-with-statment|block-scope/leave/(?:nested-block-let|verify-context-in-labelled|x-after-break)", "label"),
    (r"labeled/value-await-non-module\.js", "unexpected :"),
]
SKIPPED = [
    ("harness:$262", r"\$262"),
    ("harness:eval", r"\beval\b|\bFunction\s*\("),
    ("harness:$DONE", r"\$DONE|\basyncTest\b"),
    ("syntax:comma", r"\bfor\s*\([^;)]*,[^;)]*;"),
    ("builtins (slice 4)",
     r"\b(?:Object|Array|String|Number|Boolean|Math|JSON|Promise|Symbol|Reflect|Proxy|Map|Set|WeakMap|WeakSet"
     r"|WeakRef|Function|Date|RegExp|BigInt|ArrayBuffer|DataView|Atomics|globalThis|isNaN|isFinite|parseInt|parseFloat"
     r"|encodeURI|decodeURI|console|arguments|Int8Array|Uint8Array|Float64Array)\b"
     r"|\.prototype\b|(?:\b(?!assert\b)[\w$]+|[\])])\s*\.\s*(?!sameValue\b|notSameValue\b|throws\b)\w+\s*\("),
    ("promises (slice 5)", r"\bawait\b|\basync\b"),
    ("bitwise", r"[^&|]&[^&=]|[^|&]\|[^|=]|\^|~|<<|>>"),
]


def front(text):
    meta = re.search(r"/\*---(.*?)---\*/", text, re.S)
    body = meta.group(1) if meta else ""
    found = {}
    for key in ("features", "flags", "includes"):
        inline = re.search(rf"^{key}:\s*\[(.*?)\]", body, re.M)
        if inline:
            found[key] = [x.strip() for x in inline.group(1).split(",") if x.strip()]
            continue
        block = re.search(rf"^{key}:\s*\n((?:\s+-\s*.*\n)+)", body, re.M)
        found[key] = re.findall(r"-\s*(\S+)", block.group(1)) if block else []
    negative = re.search(r"^negative:\s*\n\s+phase:\s*(\w+)\s*\n\s+type:\s*(\w+)", body, re.M)
    found["negative"] = negative.groups() if negative else None
    return found


def code_of(text):
    text = re.sub(r"/\*.*?\*/", lambda m: " " * len(m.group(0)), text, flags=re.S)
    text = re.sub(r"//[^\n\r\u2028\u2029]*", "", text)
    return re.sub(r"'(?:\\.|[^'\\\n])*'|\"(?:\\.|[^\"\\\n])*\"", "''", text)


def denied():
    rules = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "ci/denylist.txt")
    with open(rules) as f:
        lines = [line.strip() for line in f if line.strip() and not line.startswith("#")]
    return [re.compile(line.replace("[[:<:]]", r"\b")) for line in lines]


DENIED = denied()


def judge(path, text):
    if any(rule.search(text) for rule in DENIED):
        return ("skip", "hygiene: a name ci/denylist.txt keeps out of the repo")
    meta = front(text)
    for flag in meta["flags"]:
        if flag not in FLAGS:
            return ("skip", f"flags:{flag}")
    for include in meta["includes"]:
        if include not in INCLUDES:
            return ("skip", f"includes:{include}")
    if path.endswith("_FIXTURE.js"):
        return ("skip", "fixture")
    for tag, rule in DIFFERS:
        if re.search(rule, path):
            return ("skip", tag)
    code = code_of(text)
    out = [f for f in meta["features"] if f in OUT_FEATURES]
    for feature in meta["features"]:
        if feature not in FEATURES and feature not in OUT_FEATURES:
            return ("skip", f"feature:{feature}")
    hits = sorted((m.start(), tag, word) for tag, rule, word in REFUSED for m in [re.search(rule, code)] if m)
    hits += [(len(code), "syntax:known", word) for rule, word in KNOWN_OUT if re.search(rule, path)]
    if hits or out:
        if meta["negative"]:
            return ("skip", "negative, and a left-out construct")
        if out and (not hits or OUT_FEATURES[out[0]] not in hits[0][2]):
            return ("skip", f"feature:{out[0]}")
        return ("out", hits[0][1], "|".join(dict.fromkeys(word for _, _, word in hits)))
    for tag, rule in SKIPPED:
        if re.search(rule, code):
            return ("skip", tag)
    if meta["negative"] and meta["negative"][0] not in ("parse", "runtime"):
        return ("skip", f"negative:{meta['negative'][0]}")
    return ("run", meta["negative"])


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: test262-pick.py <test262 checkout>")
    src = sys.argv[1]
    dest = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "crates/shed-script/tests/test262")
    commit = subprocess.run(["git", "-C", src, "rev-parse", "HEAD"], capture_output=True, text=True, check=True)
    shutil.rmtree(os.path.join(dest, "files"), ignore_errors=True)
    run, out, skipped = [], collections.defaultdict(list), collections.defaultdict(list)
    root = os.path.join(src, "test")
    for folder, _, names in sorted(os.walk(os.path.join(root, "language"))):
        for name in sorted(names):
            if not name.endswith(".js"):
                continue
            path = os.path.join(folder, name)
            rel = os.path.relpath(path, root)
            with open(path, encoding="utf-8") as f:
                verdict = judge(rel, f.read())
            if verdict[0] == "out" and len(out[verdict[1]]) < OUT_PER_TAG:
                out[verdict[1]].append((rel, verdict[2]))
            elif verdict[0] == "run":
                run.append(rel)
            else:
                skipped[verdict[1]].append(rel)
    for rel in run + [rel for files in out.values() for rel, _ in files]:
        target = os.path.join(dest, "files", rel)
        os.makedirs(os.path.dirname(target), exist_ok=True)
        shutil.copyfile(os.path.join(root, rel), target)
    with open(os.path.join(dest, "run.txt"), "w") as f:
        f.writelines(f"{rel}\n" for rel in run)
    with open(os.path.join(dest, "out.txt"), "w") as f:
        f.writelines(f"{word}\t{rel}\n" for tag in sorted(out) for rel, word in out[tag])
    with open(os.path.join(dest, "skipped.txt"), "w") as f:
        for tag in sorted(skipped, key=lambda t: (-len(skipped[t]), t)):
            f.write(f"# {tag}: {len(skipped[tag])}\n")
            f.writelines(f"{rel}\n" for rel in skipped[tag])
    shutil.copyfile(os.path.join(src, "LICENSE"), os.path.join(dest, "LICENSE"))
    with open(os.path.join(dest, "README.md"), "w") as f:
        f.write(
            "# test262, the subset shed's script runs\n\n"
            f"Picked by `scripts/test262-pick.py` from tc39/test262 at `{commit.stdout.strip()}`; its licence is\n"
            "`LICENSE`. `run.txt` must pass, `out.txt` must fail with the SyntaxError naming the construct, and\n"
            "`skipped.txt` lists every other file under its tag. Run by `crates/shed-script/tests/test262.rs`.\n"
        )
    print(f"run {len(run)}, out {sum(map(len, out.values()))}, skipped {sum(map(len, skipped.values()))}")


main()
