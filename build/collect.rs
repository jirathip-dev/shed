#![allow(
    dead_code,
    reason = "each build script uses its own part of the helper"
)]

use std::collections::BTreeMap;
use std::path::Path;
use std::{env, fs, io};

pub struct Row {
    pub stem: String,
    pub path: String,
}

impl Row {
    pub fn module(&self) -> String {
        format!("#[path = {:?}]\nmod {};\n", self.path, self.stem)
    }

    pub fn variant(&self) -> String {
        camel(&self.stem)
    }
}

pub fn rows(dir: &str, ext: &str) -> io::Result<Vec<Row>> {
    let dir = fs::canonicalize(dir)?;
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut rows = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == ext) {
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            let (stem, path) = (stem.into_owned(), path.display().to_string());
            rows.push(Row { stem, path });
        }
    }
    rows.sort_by(|a, b| a.stem.cmp(&b.stem));
    Ok(rows)
}

pub fn camel(stem: &str) -> String {
    let words = stem.split('_').map(|w| {
        let mut chars = w.chars();
        chars.next().map_or_else(String::new, |c| {
            c.to_uppercase().chain(chars).collect::<String>()
        })
    });
    words.collect()
}

pub fn write(file: &str, text: &str) -> io::Result<()> {
    let out = env::var_os("OUT_DIR").ok_or_else(|| io::Error::other("OUT_DIR is unset"))?;
    fs::write(Path::new(&out).join(file), text)
}

pub struct Record {
    pub file: String,
    pub variant: String,
    pub keys: BTreeMap<String, String>,
    pub prose: Vec<String>,
}

impl Record {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.keys.get(key).map(String::as_str)
    }
}

pub fn records(dir: &str, keys: &[&str], unique: &[&str]) -> Result<Vec<Record>, String> {
    let rows = rows(dir, "md").map_err(|e| format!("{dir}: {e}"))?;
    let mut records = Vec::new();
    for row in rows {
        let file = format!("{dir}/{}.md", row.stem);
        let text = fs::read_to_string(&row.path).map_err(|e| format!("{file}: {e}"))?;
        records.push(record(&file, &row.variant(), &text, keys)?);
    }
    check(records, unique)
}

pub fn record(file: &str, variant: &str, text: &str, keys: &[&str]) -> Result<Record, String> {
    let (head, prose) = text.split_once("\n\n").unwrap_or((text, ""));
    let mut found = BTreeMap::new();
    for line in head.lines() {
        let (key, value) = line
            .split_once(": ")
            .ok_or_else(|| format!("{file}: `{line}` is no `key: value` line"))?;
        if !keys.contains(&key) {
            return Err(format!("{file}: unknown key `{key}`"));
        }
        if found
            .insert(key.to_owned(), value.trim().to_owned())
            .is_some()
        {
            return Err(format!("{file}: `{key}` given twice"));
        }
    }
    let prose: Vec<String> = prose.lines().map(|l| l.trim().to_owned()).collect();
    Ok(Record {
        file: file.to_owned(),
        variant: variant.to_owned(),
        keys: found,
        prose: prose.into_iter().filter(|l| !l.is_empty()).collect(),
    })
}

pub fn check(records: Vec<Record>, unique: &[&str]) -> Result<Vec<Record>, String> {
    for key in unique {
        let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
        for r in &records {
            let value = r
                .get(key)
                .ok_or_else(|| format!("{}: no `{key}`", r.file))?;
            if let Some(other) = seen.insert(value, &r.file) {
                return Err(format!("{}: `{key}: {value}` is {other}'s too", r.file));
            }
        }
    }
    Ok(records)
}
