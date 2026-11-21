#![doc = "The agent records: built in, with an app's own row over one: `catalog <agent> <login command>`."]

use std::io::Write as _;

use shed_catalog::Catalog;

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let (agent, login) = (
        args.next().unwrap_or_default(),
        args.next().unwrap_or_default(),
    );
    let catalog = Catalog::built_in().over(&agent, &format!("login = {login:?}"))?;
    let record = catalog.by_agent(&agent).ok_or("no such agent")?;
    let line = format!(
        "{}: login {:?}, efforts {:?}, models {:?}",
        record.id, record.login, record.efforts, record.models
    );
    writeln!(std::io::stdout(), "{line}").map_err(|e| e.to_string())
}
