- `shed-proto` no longer turns on serde_json's `preserve_order` or pulls in `schemars` for its users: the
  JSON Schema derives and `openrpc()` sit behind its `schema` feature, off by default. `shed` and
  `shed-server` turn both on, so `shed schema`, `rpc.discover` and the wire JSON are unchanged.
