use std::fs;
use std::path::Path;

use anyhow::Context as _;

const FILES: [(&str, &str); 5] = [
    (
        "SKILL.md",
        include_str!("../../../skill/shed-client/SKILL.md"),
    ),
    (
        "examples/lane.rs",
        include_str!("../../../skill/shed-client/examples/lane.rs"),
    ),
    (
        "examples/events.rs",
        include_str!("../../../skill/shed-client/examples/events.rs"),
    ),
    (
        "examples/catalog.rs",
        include_str!("../../../skill/shed-client/examples/catalog.rs"),
    ),
    (
        "examples/fake.rs",
        include_str!("../../../skill/shed-client/examples/fake.rs"),
    ),
];

pub fn write(to: &Path) -> anyhow::Result<Vec<String>> {
    let dir = to.join("shed-client");
    fs::create_dir_all(dir.join("examples")).with_context(|| dir.display().to_string())?;
    let mut written = Vec::new();
    for (name, text) in FILES {
        let path = dir.join(name);
        fs::write(&path, text).with_context(|| path.display().to_string())?;
        written.push(path.display().to_string());
    }
    Ok(written)
}
