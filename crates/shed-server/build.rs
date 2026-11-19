//! Builds in every provider record in `src/providers/*.toml` and every line format in
//! `src/providers/*.rs`, so a new agent is one data file and, for the stream driver, one Rust file.

use std::path::{Path, PathBuf};
use std::{env, fs, io};

fn main() -> io::Result<()> {
    let dir = fs::canonicalize("src/providers")?;
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<io::Result<_>>()?;
    files.sort();
    let of = |ext: &str, entry: &dyn Fn(String, String) -> String| {
        let files = files
            .iter()
            .filter(|f| f.extension().is_some_and(|e| e == ext));
        let entries: Vec<String> = files
            .map(|f| {
                let name = f.file_stem().unwrap_or_default().to_string_lossy();
                entry(name.into_owned(), f.display().to_string())
            })
            .collect();
        entries.concat()
    };
    let records = of("toml", &|name, path| {
        format!("({name:?}, include_str!({path:?})), ")
    });
    let mods = of("rs", &|name, path| {
        format!("#[path = {path:?}]\nmod {name};\n")
    });
    let wires = of("rs", &|name, _| format!("({name:?}, {name}::wire), "));
    let out = env::var_os("OUT_DIR").ok_or_else(|| io::Error::other("OUT_DIR is unset"))?;
    fs::write(
        Path::new(&out).join("providers.rs"),
        format!("&[{records}]\n"),
    )?;
    let wires = format!("{mods}const WIRES: &[(&str, MakeWire)] = &[{wires}];\n");
    fs::write(Path::new(&out).join("wires.rs"), wires)
}
