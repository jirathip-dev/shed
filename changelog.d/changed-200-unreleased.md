- Protocol 3: a client on protocol 3 that sends a request field its method does not document gets
  `INVALID_PARAMS: unknown field <method>.<field>` instead of the field being dropped; clients on
  protocol 1 or 2 are served as before. shed's own client needs a protocol 3 server, and a call an older
  server lacks fails with `SHED_VERSION` naming the method or field. `just protocol` fails a change that
  drops or retypes a method, field or type in `docs/shed.openrpc.json` without raising the protocol.
