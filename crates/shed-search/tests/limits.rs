//! `outline` on a real file: its result, and each limit it can go over, with small limits.

use std::fs;

use shed_proto::{Kind, Lang, OutlineParams};
use shed_search::{Error, Limit, Limits, Over, outline};

const TEXT: &str = "fn a() {}\nfn b() {}\nfn c() {}\n";

#[test]
fn outline_reads_the_file_and_stops_at_each_limit() {
    let path = std::env::temp_dir().join(format!("shed-search-limits-{}.rs", std::process::id()));
    fs::write(&path, TEXT).unwrap();
    let p = OutlineParams {
        path: path.clone(),
        machine: None,
    };
    let run = |symbols, bytes| {
        outline(
            &p,
            &Limits {
                symbols,
                bytes,
                ..shed_search::LIMITS
            },
        )
    };
    let (fits, symbols, bytes) = (run(3, 30), run(2, 30), run(3, 29));
    fs::remove_file(&path).unwrap();
    let missing = run(3, 30);
    let at = path.display().to_string();

    let fits = fits.unwrap();
    assert_eq!(
        (fits.lang, fits.lines, fits.path.as_str()),
        (Lang::Rust, 3, at.as_str())
    );
    let names: Vec<_> = fits
        .symbols
        .iter()
        .map(|s| (s.kind, s.name.as_str(), s.start, s.end))
        .collect();
    assert_eq!(
        names,
        [
            (Kind::Fn, "a", 1, 1),
            (Kind::Fn, "b", 2, 2),
            (Kind::Fn, "c", 3, 3)
        ]
    );

    let over = |limit, max| {
        Err(Error::Limit(Over {
            limit,
            max,
            at: at.clone(),
        }))
    };
    assert_eq!(symbols, over(Limit::Symbols, 2));
    assert_eq!(bytes, over(Limit::Bytes, 29));
    assert_eq!(
        symbols.unwrap_err().to_string(),
        format!("symbols over 2 at {at}")
    );
    assert!(matches!(missing, Err(Error::Io(why)) if why.starts_with(&at)));
}
