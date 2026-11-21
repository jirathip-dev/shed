//! A hand-rolled handshake outside the client and the dispatch: the Hello::ours ban.

fn hello() -> shed_proto::Hello {
    shed_proto::Hello::ours()
}
