- The agent records live in a crate of their own, `shed-catalog`, which an app reads with no server running:
  built in, under the host's files, under the app's own rows, merged key by key. Records gain `install`,
  `token_var`, `models` (moved up from `[stream]`), `model_args`, `model_env` and `[stream] effort`, and
  `Provider::pick` turns a model and an effort into the words or variables for the driver that runs. A
  gemini record is built in, and claude's efforts include `max`. Protocol 4: `Stream.models` moved to
  `Provider.models`.
