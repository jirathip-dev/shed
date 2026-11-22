use shed_proto::{Found, Symbol};

use crate::lang;
use crate::needle::Needle;

pub(crate) struct Word {
    text: String,
    needle: Needle,
}

pub(crate) fn words(query: &str) -> Vec<Word> {
    let mut words: Vec<Word> = Vec::new();
    for text in query.split_whitespace().map(str::to_lowercase) {
        if !words.iter().any(|w| w.text == text) {
            let needle = Needle::new(&text, true);
            words.push(Word { text, needle });
        }
    }
    words
}

const REASONS: [&str; 4] = ["name is", "name has", "path has", "lines:"];

pub(crate) fn rank(words: &[Word], rel: &str, hay: &[u8]) -> Option<Found> {
    let lower = rel.to_lowercase();
    let (dirs, name) = lower.rsplit_once('/').unwrap_or(("", &lower));
    let stem = name.split('.').next().unwrap_or(name);
    let lang = lang::of(rel);
    let mut outlined: Option<Vec<Symbol>> = None;
    let mut named: Vec<(bool, Symbol)> = Vec::new();
    let mut reasons: [Vec<String>; 4] = Default::default();
    let (mut score, mut earned) = (0u32, 0u32);
    for w in words {
        let (before, word) = (score, w.text.as_str());
        let mut earn = |points, reason: usize, text: String| {
            score += points;
            if let Some(r) = reasons.get_mut(reason) {
                r.push(text);
            }
        };
        if stem == word {
            earn(40, 0, word.to_owned());
        } else if name.contains(word) {
            earn(24, 1, word.to_owned());
        }
        if dirs.contains(word) {
            let exact = dirs.split('/').any(|d| d == word);
            earn(6 + 6 * u32::from(exact), 2, word.to_owned());
        }
        let lines = count_lines(&w.needle, hay);
        if lines > 0 {
            let symbols =
                outlined.get_or_insert_with(|| lang::outline(lang, &String::from_utf8_lossy(hay)));
            let points = symbol_points(symbols, word, &mut named);
            earn(
                points + u32::try_from(lines.min(10)).unwrap_or(10),
                3,
                format!("{word} {lines}"),
            );
        }
        earned += u32::from(score > before);
    }
    if score == 0 {
        return None;
    }
    let mut why: Vec<String> = REASONS
        .iter()
        .zip(reasons)
        .filter(|(_, words)| !words.is_empty())
        .map(|(what, words)| format!("{what} {}", words.join(", ")))
        .collect();
    score *= earned;
    if is_test(rel) {
        score /= 2;
        why.push("a test".into());
    }
    named.sort_by_key(|n| !n.0);
    named.truncate(5);
    Some(Found {
        path: rel.to_owned(),
        lang,
        lines: String::from_utf8_lossy(hay).lines().count(),
        score,
        why,
        symbols: named.into_iter().map(|n| n.1).collect(),
    })
}

fn symbol_points(symbols: &[Symbol], word: &str, named: &mut Vec<(bool, Symbol)>) -> u32 {
    let mut mine: Vec<(bool, &Symbol)> = symbols
        .iter()
        .filter_map(|s| {
            let lower = s.name.to_lowercase();
            lower.contains(word).then_some((lower == word, s))
        })
        .collect();
    mine.sort_by_key(|m| !m.0);
    let mut points = 0;
    for (exact, s) in mine.into_iter().take(3) {
        points += if exact { 16 } else { 6 };
        if !named.iter().any(|n| n.1 == *s) {
            named.push((exact, s.clone()));
        }
    }
    points
}

fn count_lines(needle: &Needle, hay: &[u8]) -> usize {
    let (mut lines, mut from) = (0, 0);
    while let Some(at) = needle.find(hay, from) {
        lines += 1;
        let rest = hay.get(at..).unwrap_or_default();
        from = rest
            .iter()
            .position(|&b| b == b'\n')
            .map_or(hay.len(), |i| at + i + 1);
    }
    lines
}

fn is_test(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    path.split('/').any(|d| d == "tests")
        || name.starts_with("test_")
        || ["_test.", ".test.", ".spec."]
            .iter()
            .any(|m| name.contains(m))
}
