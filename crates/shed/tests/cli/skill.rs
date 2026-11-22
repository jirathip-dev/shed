use std::path::Path;

use crate::common::temp_dir;
use crate::shed;

#[test]
fn skill_writes_the_client_skill_and_its_examples_as_the_repo_has_them() {
    let dir = temp_dir("skill");
    let listed = shed(&["skill", "--to", dir.to_str().unwrap()]);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skill/shed-client");
    let names = [
        "SKILL.md",
        "examples/lane.rs",
        "examples/events.rs",
        "examples/catalog.rs",
        "examples/fake.rs",
    ];
    for name in names {
        let written = dir.join("shed-client").join(name);
        assert!(listed.contains(written.to_str().unwrap()), "{listed}");
        let want = std::fs::read_to_string(repo.join(name)).unwrap();
        assert_eq!(std::fs::read_to_string(&written).unwrap(), want, "{name}");
    }
    assert_eq!(listed.lines().count(), names.len());
    assert_eq!(shed(&["skill", "--to", dir.to_str().unwrap()]), listed);
}
