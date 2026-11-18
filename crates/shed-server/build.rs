//! Builds in every provider record in `src/providers/*.toml`, so a new agent is one data file.

use std::path::{Path, PathBuf};
use std::{env, fs, io};

fn main() -> io::Result<()> {
    let dir = fs::canonicalize("src/providers")?;
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<io::Result<_>>()?;
    files.retain(|f| f.extension().is_some_and(|e| e == "toml"));
    files.sort();
    let entries: Vec<String> = files
        .iter()
        .map(|f| {
            let name = f.file_stem().unwrap_or_default().to_string_lossy();
            format!("({name:?}, include_str!({:?}))", f.display().to_string())
        })
        .collect();
    let out = env::var_os("OUT_DIR").ok_or_else(|| io::Error::other("OUT_DIR is unset"))?;
    let list = format!("&[{}]\n", entries.join(", "));
    fs::write(Path::new(&out).join("providers.rs"), list)
}
