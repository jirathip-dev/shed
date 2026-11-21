#![doc = "Follows the events of the terminals labelled `app=example`, resuming where a closed stream stopped, until the server is gone: `events <socket>`."]

use std::io::Write as _;
use std::path::PathBuf;

use shed_client::proto::EventsParams;
use shed_client::{Error, Shed, Streamed};

fn main() -> Result<(), Error> {
    let socket = std::env::args_os().nth(1).unwrap_or_default();
    let shed = Shed::at(socket);
    let labels: std::collections::BTreeMap<_, _> =
        [("app".to_owned(), "example".to_owned())].into();
    let mut since = None;
    loop {
        let mut stream = shed.events(&EventsParams {
            labels: labels.clone(),
            since,
        })?;
        for item in stream.by_ref() {
            let line = match item {
                Streamed::Item(event) => format!("{} {:?}", event.name, event.kind),
                Streamed::Reset(seq) | Streamed::Lagged(seq) => format!("lost events up to {seq}"),
                Streamed::Skipped(line) => format!("an event this build does not know: {line}"),
                Streamed::Quiet => continue,
            };
            writeln!(std::io::stdout(), "{line}").map_err(|source| Error::Io {
                socket: PathBuf::new(),
                source,
            })?;
        }
        since = stream.since().or(since);
    }
}
