- `hello` checks a protocol range, not an exact match: shed speaks protocol 2 and still serves 1, answering
  a protocol 1 client in protocol 1's shapes. Each side's `hello` carries `oldest`, and `SHED_VERSION`
  names both ranges and which side to update.
