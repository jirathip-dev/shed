#![doc = "Starts an ACP lane, prompts it and waits for its turn to end: `lane <socket> <agent-id> <cwd> <prompt>`."]

use std::env::args_os;
use std::io::Write as _;
use std::path::PathBuf;

use shed_client::proto::{Policy, RunParams};
use shed_client::{Calls as _, Error, Shed};

fn main() -> Result<(), Error> {
    let mut args = args_os().skip(1).map(|a| a.to_string_lossy().into_owned());
    let mut arg = || args.next().unwrap_or_default();
    let (socket, agent, cwd, prompt) = (arg(), arg(), arg(), arg());
    let shed = Shed::at(socket);
    let name = "example-lane".to_owned();
    shed.run(&RunParams {
        name: Some(name.clone()),
        acp: Some(agent),
        cwd: PathBuf::from(cwd),
        policy: Some(Policy::Allow),
        effort: Some("high".to_owned()),
        ..RunParams::default()
    })?;
    shed.ready(&name, 60.0)?;
    shed.prompt(&name, &prompt)?;
    let done = shed.wait_state(&name, &["idle", "blocked", "done"], 600.0)?;
    let screen = shed.screen(&name)?;
    shed.close(&name)?;
    let outcome = done.outcome;
    let shown = writeln!(std::io::stdout(), "{outcome:?}\n{screen}");
    shown.map_err(|source| Error::Io {
        socket: PathBuf::new(),
        source,
    })
}
