//! Builds in every line format in `src/providers/*.rs`, so a stream driver's agent is one Rust file.

#[path = "../../build/collect.rs"]
mod collect;

fn main() -> std::io::Result<()> {
    let rows = collect::rows("src/providers", "rs")?;
    let mods: String = rows.iter().map(collect::Row::module).collect();
    let wires: Vec<String> = rows
        .iter()
        .map(|r| format!("({:?}, {}::wire), ", r.stem, r.stem))
        .collect();
    let wires = wires.concat();
    let wires = format!("{mods}const WIRES: &[(&str, MakeWire)] = &[{wires}];\n");
    collect::write("wires.rs", &wires)
}
