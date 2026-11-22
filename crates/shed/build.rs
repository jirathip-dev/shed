//! Builds in every command in `src/cmd/*.rs` and every `shed doctor` row in `src/doctor/*.rs`, one file each.

#[path = "../../build/collect.rs"]
mod collect;

use std::fs;

fn main() -> std::io::Result<()> {
    collect::write("cmd.rs", &commands()?)?;
    collect::write("doctor.rs", &doctor()?)
}

fn commands() -> std::io::Result<String> {
    let rows = collect::rows("src/cmd", "rs")?;
    let mods: String = rows.iter().map(collect::Row::module).collect();
    let (mut variants, mut on, mut run) = (Vec::new(), Vec::new(), Vec::new());
    for row in &rows {
        let (variant, stem) = (row.variant(), &row.stem);
        variants.push(format!("{variant}(Box<{stem}::Args>),\n"));
        if fs::read_to_string(&row.path)?.contains("pub fn on(") {
            on.push(format!("Command::{variant}(a) => {stem}::on(a),\n"));
        } else {
            on.push(format!("Command::{variant}(_) => {stem}::ON,\n"));
        }
        let text = fs::read_to_string(&row.path)?;
        let call = format!("{stem}::run(cx, *a)");
        let call = if text.contains("-> ExitCode {") {
            format!("Ok({call})")
        } else {
            call
        };
        run.push(format!("Command::{variant}(a) => {call},\n"));
    }
    let (variants, on, run) = (variants.concat(), on.concat(), run.concat());
    Ok(format!(
        "{mods}\n#[derive(clap::Subcommand)]\npub enum Command {{\n{variants}}}\n\n\
         pub fn on(command: &Command) -> crate::cx::On {{\nmatch command {{\n{on}}}\n}}\n\n\
         pub fn run(cx: &crate::cx::Cx<'_>, command: Command) -> anyhow::Result<shed_server::runner::ExitCode> {{\n\
         match command {{\n{run}}}\n}}\n"
    ))
}

fn doctor() -> std::io::Result<String> {
    let rows = collect::rows("src/doctor", "rs")?;
    let mut mods = Vec::new();
    let mut list = Vec::new();
    for row in &rows {
        let name = row.stem.trim_start_matches(|c: char| c.is_ascii_digit());
        let name = name.trim_start_matches('_');
        mods.push(format!("#[path = {:?}]\nmod {name};\n", row.path));
        list.push(format!("{name}::rows"));
    }
    let (mods, list) = (mods.concat(), list.join(", "));
    Ok(format!(
        "{mods}\nconst ROWS: &[fn(&Checks<'_>) -> Vec<Row>] = &[{list}];\n"
    ))
}
