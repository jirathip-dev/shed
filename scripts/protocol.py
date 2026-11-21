#!/usr/bin/env python3
"""A change that drops or retypes a method, field or type in docs/shed.openrpc.json raises PROTOCOL (#200).

`protocol.py [--base REV]` compares the document at REV (default HEAD) with the working tree's;
`protocol.py DIR` compares DIR/base.json with DIR/head.json. A field's or type's shape is its schema's
`type`, `$ref`, `format` and `items`; descriptions and defaults may change freely, and additions never
need a raise. PROTOCOL is the document's `info.x-protocol`.
"""

import json
import pathlib
import subprocess
import sys

DOC = "docs/shed.openrpc.json"


def shape(schema):
    if not isinstance(schema, dict):
        return schema
    return {k: schema[k] for k in ("type", "$ref", "format", "items") if k in schema}


def surface(doc):
    out = {}
    for method in doc.get("methods", []):
        name = method["name"]
        out[f"method {name}"] = {}
        for param in method.get("params", []):
            out[f"field {name}.{param['name']}"] = shape(param.get("schema", {}))
        out[f"result {name}"] = shape(method.get("result", {}).get("schema", {}))
    for name, schema in doc.get("components", {}).get("schemas", {}).items():
        out[f"type {name}"] = shape(schema)
        for field, sub in schema.get("properties", {}).items():
            out[f"field {name}.{field}"] = shape(sub)
    return out


def docs(args):
    if args and args[0] != "--base":
        folder = pathlib.Path(args[0])
        return [json.loads((folder / f).read_text()) for f in ("base.json", "head.json")]
    rev = args[1] if len(args) > 1 else "HEAD"
    base = subprocess.run(["git", "show", f"{rev}:{DOC}"], capture_output=True, text=True)
    if base.returncode != 0:
        return None
    return [json.loads(base.stdout), json.loads(pathlib.Path(DOC).read_text())]


def main(args):
    pair = docs(args)
    if pair is None:
        print(f"protocol: no {DOC} at the base; nothing to compare")
        return 0
    base, head = pair
    was, now = (d.get("info", {}).get("x-protocol") for d in (base, head))
    old, new = surface(base), surface(head)
    broken = [f"{k}: {'gone' if k not in new else 'retyped'}" for k in old if new.get(k) != old[k]]
    if broken and was == now:
        print(f"protocol: these break protocol {now}; raise PROTOCOL in crates/shed-proto/src/lib.rs:")
        print("\n".join(f"  {line}" for line in broken))
        return 1
    print(f"protocol: {len(broken)} breaking change(s), protocol {was} -> {now}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
