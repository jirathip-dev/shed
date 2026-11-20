//! Each `tests/golden/<name>.outline` is the text outline of `<name>.in`, its language by `<name>`.

use std::fs;
use std::path::Path;

use shed_proto::OutlineResult;
use shed_search::{lang, text};

#[test]
fn outlines_match_their_goldens() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let mut seen = 0;
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "outline") {
            continue;
        }
        let name = path.file_stem().unwrap().to_str().unwrap().to_owned();
        let input = fs::read_to_string(dir.join(format!("{name}.in"))).unwrap();
        let lang = lang::of(&name);
        let result = OutlineResult {
            lines: input.lines().count(),
            symbols: lang::outline(lang, &input),
            path: name.clone(),
            lang,
        };
        let want = fs::read_to_string(&path).unwrap();
        assert_eq!(text::outline(&result), want, "{name}");
        seen += 1;
    }
    assert_eq!(seen, 6);
}
