wire: emit
params: EmitParams

Emit one of an ACP terminal's structured events (`turn`, `text`, `tool_call`, `plan`, `mode`, `approval_request`, `approved`, `reply`); its bridge (`shed run --acp`) calls this.
Other events are refused.
