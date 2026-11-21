- `run` takes a `model`, an `effort` and an `env` (`shed run --model --effort --env K=V`): the server turns
  the model and effort into the agent record's words or variables for the driver it picks, and gives the
  variables to the program. Their values stay in the server's memory: not in `get`, `agents`, events, logs
  or the saved state. `warm` takes no `env`, and `model` and `effort` need `acp`.
