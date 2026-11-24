use std::fs;
use std::io::{self, Write as _};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use shed_proto::BUILD;
use shed_proto::flow::{Class, Disagreement, FlowReport, Report};

const MAX_BYTES: u64 = 10 << 20;
const WEEK: u64 = 7 * 86_400;

fn lines_file(state: &Path) -> PathBuf {
    state.with_file_name("flow-shadow.jsonl")
}

fn counts_file(state: &Path) -> PathBuf {
    state.with_file_name("flow-counts.json")
}

fn open(path: &Path, append: bool) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.create(true).write(true).mode(0o600);
    options.append(append).truncate(!append).open(path)
}

pub(super) fn counts(state: &Path) -> Vec<FlowReport> {
    let kept = fs::read(counts_file(state)).ok();
    kept.and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub(super) fn write(
    state: &Path,
    lines: &[Disagreement],
    counts: Option<&[FlowReport]>,
) -> io::Result<()> {
    if let Some(counts) = counts {
        open(&counts_file(state), false)?.write_all(&serde_json::to_vec(counts)?)?;
    }
    if lines.is_empty() {
        return Ok(());
    }
    let mut text = Vec::new();
    for line in lines {
        serde_json::to_writer(&mut text, line)?;
        text.push(b'\n');
    }
    let path = lines_file(state);
    open(&path, true)?.write_all(&text)?;
    if fs::metadata(&path)?.len() > MAX_BYTES {
        let all = fs::read_to_string(&path)?;
        let half = all.lines().count() / 2;
        let kept: Vec<&str> = all.lines().skip(half).chain([""]).collect();
        open(&path, false)?.write_all(kept.join("\n").as_bytes())?;
    }
    Ok(())
}

pub(crate) fn report(state: &Path, since: Option<u64>, now: u64) -> Report {
    let since = since.unwrap_or(now.saturating_sub(WEEK));
    let mut flows = super::Runner::new(state).reports();
    let text = fs::read_to_string(lines_file(state)).unwrap_or_default();
    let lines = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Disagreement>(l).ok());
    for d in lines.filter(|d| d.at >= since) {
        let Some(flow) = flows.iter_mut().find(|f| f.flow == d.flow) else {
            continue;
        };
        let same = |c: &&mut Class| {
            (c.kind, &c.row, &c.table, &c.code) == (d.kind, &d.row, &d.table, &d.code)
        };
        match flow.disagreements.iter_mut().find(same) {
            Some(class) => (class.count, class.last) = (class.count + 1, d.at),
            None => flow.disagreements.push(Class {
                kind: d.kind,
                row: d.row,
                table: d.table,
                code: d.code,
                count: 1,
                first: d.at,
                last: d.at,
                sample: d.id,
            }),
        }
    }
    Report {
        build: BUILD.to_owned(),
        since,
        flows,
    }
}
