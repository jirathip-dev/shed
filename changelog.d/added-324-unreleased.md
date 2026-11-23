- `shed machine add --host-key --identity --replace` (`machine.add`'s `host_key`, `identity` and `replace`): a
  machine takes `ssh://user@host:port`, dials only its pinned host key and offers only its key file; another key
  holds its link down with `KEY_CHANGED` until a re-pin with `--replace`. Protocol 6.
