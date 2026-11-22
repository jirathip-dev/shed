wire: events
params: EventsParams
stream: Event

Stream events.
After the response the server writes one event per line (see Events) for the terminals carrying every one of `labels`, first those after `since` still kept (the last 4,096 events, and each terminal's last 256 `text` chunks before them), and never closes the connection; closing it unsubscribes.
When events after `since` are gone it first writes `{"jsonrpc":"2.0","method":"reset","params":{"seq":N}}`: those up to `N` are lost.
A client more than 4,096 events behind gets `{"jsonrpc":"2.0","method":"lagged","params":{"seq":N}}` and the connection is closed; it resumes with `since`.
A connection that sent `heartbeat` gets `{"jsonrpc":"2.0","method":"heartbeat"}` lines when idle.
