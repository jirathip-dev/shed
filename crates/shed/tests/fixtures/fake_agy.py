"""Antigravity's ACP server as a login sees it: `authenticate` prints the sign-in link, then keeps a token
under GEMINI_HOME only with file storage forced, as agy_acp_server 1.5.2 does."""

import json
import os
import sys

for line in sys.stdin:
    msg = json.loads(line)
    result = {}
    if msg["method"] == "authenticate":
        print("Open the following link to authenticate the ACP server: https://example.test/auth", file=sys.stderr)
        if os.environ.get("AGY_ACP_FORCE_FILE_STORAGE") != "1":
            reply = {"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32000, "message": "keychain"}}
            print(json.dumps(reply), flush=True)
            continue
        folder = os.path.join(os.environ["GEMINI_HOME"], "antigravity-acp")
        os.makedirs(folder, exist_ok=True)
        with open(os.path.join(folder, "acp_token.json"), "w") as f:
            f.write('{"refresh_token": "fake"}')
    print(json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": result}), flush=True)
