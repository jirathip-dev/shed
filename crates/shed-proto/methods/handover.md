wire: handover
params: HandoverParams
result: Handover

Hand over to the binary the server was started from, which must be `build`: after the same check as `handover.check` it answers, waits up to 10 s for the other requests in flight, then replaces itself in place on the same process id, keeping the socket and every running terminal (screen, children and exit statuses).
Every connection is closed: a closed connection means reconnect and ask again; `events` with `since` gets `reset`.
A refused or failed handover leaves the server serving as it was.
