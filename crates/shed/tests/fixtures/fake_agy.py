"""Antigravity's ACP server as a login sees it: `authenticate` prints the sign-in link, then keeps a token
under GEMINI_HOME only with file storage forced, as agy_acp_server 1.5.2 does."""

import json
import os
import sys

for line in sys.stdin:
    msg = json.loads(line)
    result = {}
    if msg["method"] == "authenticate":
        print("Open the following link to authenticate the ACP server: https://accounts.google.com/o/oauth2/v2/auth?redirect_uri=http%3A%2F%2F127.0.0.1%3A57865%2Fauth%2Faicode&access_type=offline&scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fcloud-platform%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fuserinfo.email%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fuserinfo.profile&code_challenge=Xk3vQ9mB2rT7wLpZ8nY4cF6hJ1sD5gA0eU2iO3kP7qW&code_challenge_method=S256&state=9f2c4e7a1b3d5f6e8a0c2b4d6f8e1a3c&response_type=code&client_id=123456789012-a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6.apps.googleusercontent.com", file=sys.stderr)
        if os.environ.get("AGY_ACP_FORCE_FILE_STORAGE") != "1":
            reply = {"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32000, "message": "keychain"}}
            print(json.dumps(reply), flush=True)
            continue
        folder = os.path.join(os.environ["GEMINI_HOME"], "antigravity-acp")
        os.makedirs(folder, exist_ok=True)
        with open(os.path.join(folder, "acp_token.json"), "w") as f:
            f.write('{"refresh_token": "fake"}')
    print(json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": result}), flush=True)
