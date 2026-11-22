//! Builds in every line format in `src/providers/*.rs` and every handler in `src/methods/*.rs`, one file each.

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
    collect::write("wires.rs", &wires)?;
    collect::write("methods.rs", &methods()?)
}

fn methods() -> std::io::Result<String> {
    let rows = collect::rows("src/methods", "rs")?;
    let mods: String = rows.iter().map(collect::Row::module).collect();
    let mut arms = Vec::new();
    for row in &rows {
        let text = std::fs::read_to_string(&row.path)?;
        let wait = if text.contains("async fn call(") {
            ".await"
        } else {
            ""
        };
        let (variant, stem) = (row.variant(), &row.stem);
        let call = format!("{stem}::call(cx){wait}");
        let call = if text.contains("-> Value {") {
            format!("Ok({call})")
        } else {
            call
        };
        arms.push(format!("Method::{variant} => {call},\n"));
    }
    let arms = arms.concat();
    Ok(format!(
        "mod methods {{\nuse crate::call::{{Call, Reply}};\nuse crate::proto::Method;\n{mods}\n\
         pub(super) async fn call(cx: &mut Call<'_>) -> Reply {{\nmatch cx.method {{\n{arms}}}\n}}\n}}\n"
    ))
}
