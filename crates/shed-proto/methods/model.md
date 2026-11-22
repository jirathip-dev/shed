wire: acp.model
params: ModelParams
on: name
input: yes

Switch an ACP terminal's model to one of its `session.models`: its bridge asks the agent (`session/set_config_option`, the `model` option) and emits `session` with the new `model`.
