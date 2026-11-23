"""Checks the flow tables in tables.md the way the build step of slice 1 will: no (state, fact) pair without
exactly one row, every row whole, every state reachable. Exit 1 with one line per fault."""

import re
import sys

SOURCES_HEAD = "| source | is |"
FACTS_HEAD = "| fact | source | read as |"
ROWS_HEAD = "| id | state | fact | next | do | owner | within | or |"
OWNERS = {"shed", "program", "caller"}


def cells(line):
    return [c.strip() for c in line.strip().strip("|").split("|")]


def table(lines, head):
    out, on = [], False
    for line in lines:
        if line.strip() == head:
            on = True
        elif on and line.startswith("|"):
            if not set(line.strip()) <= set("|- "):
                out.append(cells(line))
        elif on:
            break
    return out


def names(cell):
    return [n.strip() for n in cell.split(",") if n.strip()]


def check(flow, lines, sources):
    faults = []
    head = {k: v.split() for k, v in (l.split(": ", 1) for l in lines if re.match(r"^(states|start|rest): ", l))}
    states, rest = head.get("states", []), set(head.get("rest", []))
    start = head.get("start", [""])[0]
    facts = {}
    for fact, source, _ in table(lines, FACTS_HEAD):
        if source not in sources:
            faults.append(f"fact {fact}: unknown source {source}")
        if fact in facts:
            faults.append(f"fact {fact}: listed twice")
        facts[fact] = source
    if facts.get("gave_up") != "clock":
        faults.append("no gave_up fact from the clock")
    if start not in states or not rest <= set(states):
        faults.append("start or rest names an unknown state")
    seen, ids, edges = {}, set(), {s: set() for s in states}
    rows = table(lines, ROWS_HEAD)
    for row in rows:
        if len(row) != 8:
            faults.append(f"{row[0]}: {len(row)} cells")
            continue
        rid, state, fact, nxt, do, owner, within, alt = row
        if rid in ids:
            faults.append(f"{rid}: id used twice")
        ids.add(rid)
        clock = (owner, within, alt)
        kept = clock == ("=", "=", "=")
        if kept and nxt != "=":
            faults.append(f"{rid}: moves to {nxt} without owner, within and or")
        if not kept:
            timed = within.isdigit()
            if owner not in OWNERS:
                faults.append(f"{rid}: owner {owner}")
            if not (timed or within == "rest" or re.fullmatch(r"every \d+", within)):
                faults.append(f"{rid}: within {within}")
            if timed == (alt == "-"):
                faults.append(f"{rid}: a timed row names its `or`, another has `-`")
            if within.startswith("every") and do in ("-", "never"):
                faults.append(f"{rid}: `every` with nothing to run")
        if do == "never" and (nxt != "=" or not kept):
            faults.append(f"{rid}: `never` moves or sets a clock")
        for s in names(state):
            if s not in states:
                faults.append(f"{rid}: unknown state {s}")
                continue
            to = s if nxt == "=" else nxt
            if to not in states:
                faults.append(f"{rid}: unknown next {nxt}")
                continue
            if not kept and not within.isdigit() and to not in rest and fact != "gave_up":
                faults.append(f"{rid}: {to} is no resting state, so the row needs a time")
            if do != "never":
                edges[s].add(to)
            for f in names(fact):
                if f not in facts:
                    faults.append(f"{rid}: unknown fact {f}")
                elif (s, f) in seen:
                    faults.append(f"({s}, {f}): rows {seen[(s, f)]} and {rid}")
                else:
                    seen[(s, f)] = rid
    for s in states:
        for f in facts:
            if (s, f) not in seen:
                faults.append(f"gap: ({s}, {f}) has no row")
    reached, todo = {start}, [start]
    while todo:
        for to in edges.get(todo.pop(), ()):
            if to not in reached:
                reached.add(to)
                todo.append(to)
    for s in states:
        if s not in reached:
            faults.append(f"state {s} cannot be reached from {start}")
        if s not in rest and not (edges[s] - {s}):
            faults.append(f"state {s} has no way out")
    pairs = len(states) * len(facts)
    print(f"{flow}: {len(states)} states x {len(facts)} facts = {pairs} pairs, {len(rows)} rows, " + ("no gap" if not faults else f"{len(faults)} faults"))
    return pairs, len(rows), [f"{flow}: {x}" for x in faults]


def main(path):
    text = open(path, encoding="utf-8").read().splitlines()
    sources = {row[0].strip("`") for row in table(text, SOURCES_HEAD)}
    sections, name = {}, None
    for line in text:
        if line.startswith("## "):
            name = line[3:].strip()
            sections[name] = []
        elif name:
            sections[name].append(line)
    flows = {n: l for n, l in sections.items() if any(x.strip() == ROWS_HEAD for x in l)}
    pairs = rows = 0
    faults = []
    for flow, lines in flows.items():
        p, r, f = check(flow, lines, sources)
        pairs, rows, faults = pairs + p, rows + r, faults + f
    print(f"{pairs} pairs, {rows} rows, {len(faults)} gaps" if not faults else f"{len(faults)} faults")
    for fault in faults:
        print(fault, file=sys.stderr)
    return 1 if faults else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
