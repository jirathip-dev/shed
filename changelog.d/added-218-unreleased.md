- `shed script -f <file>` and `shed script -` send a program to the server and print its logs and result;
  `--check` only parses and compiles it, and `shed --on <machine> script` runs it there over a connection
  of its own that ends with the run. `shed connect` now ends when whoever reads its output is gone, so a
  script whose caller dropped the SSH session stops. `skill/script/` is the skill doc, with the methods list
  generated from the protocol.
