//! Builds in every record in `providers/*.toml`, so a new agent is one data file.

use std::path::{Path, PathBuf};
use std::{env, fs, io};

fn main() -> io::Result<()> {
    let dir = fs::canonicalize("providers")?;
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<io::Result<_>>()?;
    files.sort();
    let records: Vec<String> = files
        .iter()
        .filter(|f| f.extension().is_some_and(|e| e == "toml"))
        .map(|f| {
            let name = f.file_stem().unwrap_or_default().to_string_lossy();
            format!("({name:?}, include_str!({:?})), ", f.display().to_string())
        })
        .collect();
    let out = env::var_os("OUT_DIR").ok_or_else(|| io::Error::other("OUT_DIR is unset"))?;
    fs::write(
        Path::new(&out).join("providers.rs"),
        format!("&[{}]\n", records.concat()),
    )
}
