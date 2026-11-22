wire: account.use
params: AccountUse
result: AccountMoves

Make a named, ready account the provider's preferred one, which `auto` picks first while it is ready and not limited, and move every live ACP or stream lane of the provider on `auto` or `default` to it: the lane stops its agent (a busy one once its tool call ends), copies the session's `session_files` and loads the session on the account, now on `auto`; a lane that was mid-turn or rate-limited gets one prompt to go on. Refused for `default`, an unknown, signed-out, limited or set-up account. Without `name` it only drops the preference.
