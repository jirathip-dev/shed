- `shed serve` can hand over to a new build in place: the `handover` method checks the binary
  (`handover.check`), then replaces the server on the same process id, keeping the socket and every
  running terminal, ACP sessions included, with its screen, children and exit statuses. Clients see their
  connection close and reconnect; an `events` reader with an older `since` gets `reset`. A refused check or
  a failed exec leaves the server serving as it was.
