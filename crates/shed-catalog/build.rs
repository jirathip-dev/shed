//! Builds in every record in `providers/*.toml`, so a new agent is one data file.

#[path = "../../build/collect.rs"]
mod collect;

fn main() -> std::io::Result<()> {
    let records: Vec<String> = collect::rows("providers", "toml")?
        .iter()
        .map(|r| format!("({:?}, include_str!({:?})), ", r.stem, r.path))
        .collect();
    collect::write("providers.rs", &format!("&[{}]\n", records.concat()))
}
