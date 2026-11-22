wire: heartbeat

Keep the connection alive.
Once a connection sends one, the server closes it when nothing comes from it for 30 s (two missed beats of 15 s), and writes a `heartbeat` line every 15 s into its idle `events` stream.
A write the client does not take within 30 s also closes any connection.
