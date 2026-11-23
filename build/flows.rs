#![allow(
    dead_code,
    reason = "build.rs writes the code from it, the flows test the chart"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

pub const SOURCES: [&str; 12] = [
    "call", "process", "program", "record", "ssh", "link", "list", "stats", "store", "runs",
    "there", "clock",
];
const OWNERS: [&str; 3] = ["shed", "program", "caller"];
const FACTS_HEAD: &str = "| fact | source | read as |";
const ROWS_HEAD: &str = "| id | state | fact | next | do | owner | within | or |";

pub struct Row {
    pub id: String,
    pub states: Vec<String>,
    pub facts: Vec<String>,
    pub next: String,
    pub act: String,
    pub owner: String,
    pub within: String,
    pub or: String,
}

pub struct Flow {
    pub name: String,
    pub text: String,
    pub states: Vec<String>,
    pub start: String,
    pub rest: Vec<String>,
    pub facts: Vec<(String, String)>,
    pub rows: Vec<Row>,
}

fn table(text: &str, head: &str) -> Vec<Vec<String>> {
    let mut lines = text.lines().skip_while(|l| l.trim() != head).skip(1);
    let mut out = Vec::new();
    for line in lines.by_ref().take_while(|l| l.starts_with('|')) {
        if !line.chars().all(|c| "|- ".contains(c)) {
            let cells = line.trim().trim_matches('|').split('|');
            out.push(cells.map(|c| c.trim().to_owned()).collect());
        }
    }
    out
}

fn names(cell: &str) -> Vec<String> {
    let names = cell.split(',').map(str::trim).filter(|n| !n.is_empty());
    names.map(str::to_owned).collect()
}

fn word(name: &str, extra: char) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == extra)
}

pub fn camel(name: &str) -> String {
    let words = name.split(['_', '-']).map(|w| {
        let mut chars = w.chars();
        chars.next().map_or_else(String::new, |c| {
            c.to_uppercase().chain(chars).collect::<String>()
        })
    });
    words.collect()
}

fn row(cells: &[String], faults: &mut Vec<String>) -> Option<Row> {
    let [id, state, fact, next, act, owner, within, or] = cells else {
        faults.push(format!("{}: {} cells", cells.join(" | "), cells.len()));
        return None;
    };
    let id_ok = id.starts_with(|c: char| c.is_ascii_uppercase())
        && id.chars().all(|c| c.is_ascii_alphanumeric());
    if !id_ok {
        faults.push(format!("{id}: a row id is letters then digits"));
    }
    Some(Row {
        id: id.clone(),
        states: names(state),
        facts: names(fact),
        next: next.clone(),
        act: act.clone(),
        owner: owner.clone(),
        within: within.clone(),
        or: or.clone(),
    })
}

fn head(text: &str) -> BTreeMap<&str, &str> {
    let lines = text.lines().take_while(|l| !l.trim().is_empty());
    lines.filter_map(|l| l.split_once(": ")).collect()
}

pub fn parse(name: &str, text: &str) -> (Flow, Vec<String>) {
    let mut faults = Vec::new();
    let keys = head(text);
    let list = |key: &str| {
        keys.get(key)
            .map_or_else(Vec::new, |v| names(&v.replace(' ', ",")))
    };
    let (states, rest) = (list("states"), list("rest"));
    let start = list("start").concat();
    let facts: Vec<(String, String)> = table(text, FACTS_HEAD)
        .into_iter()
        .filter_map(|c| Some((c.first()?.clone(), c.get(1)?.clone())))
        .collect();
    let rows = table(text, ROWS_HEAD);
    let rows = rows.iter().filter_map(|c| row(c, &mut faults)).collect();
    let flow = Flow {
        name: name.to_owned(),
        text: text.to_owned(),
        states,
        start,
        rest,
        facts,
        rows,
    };
    faults.extend(check(&flow));
    (
        flow,
        faults
            .into_iter()
            .map(|f| format!("flows/{name}.md: {f}"))
            .collect(),
    )
}

fn clock(r: &Row, faults: &mut Vec<String>) -> bool {
    let id = &r.id;
    let kept = [&r.owner, &r.within, &r.or].iter().all(|c| *c == "=");
    if kept && r.next != "=" {
        faults.push(format!(
            "{id}: moves to {} without owner, within and or",
            r.next
        ));
    }
    if !kept {
        let timed = r.within.parse::<u32>().is_ok();
        let every = r
            .within
            .strip_prefix("every ")
            .is_some_and(|n| n.parse::<u32>().is_ok());
        if !OWNERS.contains(&r.owner.as_str()) {
            faults.push(format!("{id}: owner {}", r.owner));
        }
        if !(timed || every || r.within == "rest") {
            faults.push(format!("{id}: within {}", r.within));
        }
        if timed == (r.or == "-") || !(r.or == "-" || word(&r.or, '-')) {
            faults.push(format!(
                "{id}: a timed row names its `or` in kebab-case, another has `-`"
            ));
        }
        if every && ["-", "never"].contains(&r.act.as_str()) {
            faults.push(format!("{id}: `every` with nothing to run"));
        }
    }
    if r.act == "never" && (r.next != "=" || !kept) {
        faults.push(format!("{id}: `never` moves or sets a clock"));
    }
    let code = r.act.strip_prefix("refuse ");
    let code_ok =
        code.is_some_and(|c| word(&c.to_ascii_lowercase(), '_') && c == c.to_ascii_uppercase());
    if !(["-", "never"].contains(&r.act.as_str())
        || code_ok
        || (code.is_none() && word(&r.act, '-')))
    {
        faults.push(format!("{id}: do {}", r.act));
    }
    kept
}

fn check(flow: &Flow) -> Vec<String> {
    let mut faults = Vec::new();
    let (states, facts) = (&flow.states, &flow.facts);
    for s in states.iter().chain(facts.iter().map(|f| &f.0)) {
        if !word(s, '_') {
            faults.push(format!("{s}: a state or fact is snake_case"));
        }
    }
    let mut known = BTreeSet::new();
    for (fact, source) in facts {
        if !SOURCES.contains(&source.as_str()) {
            faults.push(format!("fact {fact}: unknown source {source}"));
        }
        if !known.insert(fact.as_str()) {
            faults.push(format!("fact {fact}: listed twice"));
        }
    }
    if !facts.iter().any(|(f, s)| f == "gave_up" && s == "clock") {
        faults.push("no gave_up fact from the clock".to_owned());
    }
    if !states.contains(&flow.start) || !flow.rest.iter().all(|s| states.contains(s)) {
        faults.push("start or rest names an unknown state".to_owned());
    }
    let mut seen: BTreeMap<(&str, &str), &str> = BTreeMap::new();
    let mut ids = BTreeSet::new();
    let mut edges: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for r in &flow.rows {
        let id = r.id.as_str();
        if !ids.insert(id) {
            faults.push(format!("{id}: id used twice"));
        }
        let kept = clock(r, &mut faults);
        for s in &r.states {
            if !states.contains(s) {
                faults.push(format!("{id}: unknown state {s}"));
                continue;
            }
            let to = if r.next == "=" { s } else { &r.next };
            if !states.contains(to) {
                faults.push(format!("{id}: unknown next {to}"));
                continue;
            }
            let timed = r.within.parse::<u32>().is_ok();
            let fact_gave_up = r.facts.iter().all(|f| f == "gave_up");
            if !kept && !timed && !flow.rest.contains(to) && !fact_gave_up {
                faults.push(format!(
                    "{id}: {to} is no resting state, so the row needs a time"
                ));
            }
            if r.act != "never" {
                edges.entry(s).or_default().insert(to);
            }
            for f in &r.facts {
                if !known.contains(f.as_str()) {
                    faults.push(format!("{id}: unknown fact {f}"));
                } else if let Some(other) = seen.insert((s, f), id) {
                    faults.push(format!("({s}, {f}): rows {other} and {id}"));
                }
            }
        }
    }
    for s in states {
        for (f, _) in facts {
            if !seen.contains_key(&(s.as_str(), f.as_str())) {
                faults.push(format!("gap: ({s}, {f}) has no row"));
            }
        }
    }
    let mut reached = BTreeSet::from([flow.start.as_str()]);
    let mut todo = vec![flow.start.as_str()];
    while let Some(s) = todo.pop() {
        for to in edges.get(s).into_iter().flatten() {
            if reached.insert(to) {
                todo.push(to);
            }
        }
    }
    for s in states {
        if !reached.contains(s.as_str()) {
            faults.push(format!("state {s} cannot be reached from {}", flow.start));
        }
        let out = edges
            .get(s.as_str())
            .is_some_and(|e| e.iter().any(|to| to != s));
        if !flow.rest.contains(s) && !out {
            faults.push(format!("state {s} has no way out"));
        }
    }
    faults
}

pub fn load(dir: &Path) -> Result<Vec<Flow>, String> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|e| e == "md") {
            files.push(path);
        }
    }
    files.sort();
    let (mut flows, mut faults) = (Vec::new(), Vec::new());
    for path in files {
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let (flow, found) = parse(&name, &text);
        flows.push(flow);
        faults.extend(found);
    }
    if faults.is_empty() {
        Ok(flows)
    } else {
        Err(faults.join("\n"))
    }
}

fn actions(flow: &Flow) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for r in &flow.rows {
        let plain = !["-", "never"].contains(&r.act.as_str()) && !r.act.starts_with("refuse ");
        if plain && !out.contains(&r.act.as_str()) {
            out.push(&r.act);
        }
    }
    out
}

fn variants<'a>(items: impl Iterator<Item = (&'a str, String)>) -> String {
    let lines = items.map(|(name, doc)| format!("    #[doc = {doc:?}]\n    {},\n", camel(name)));
    lines.collect::<Vec<_>>().concat()
}

fn alt(prefix: &str, items: &[String]) -> String {
    let arms: Vec<String> = items
        .iter()
        .map(|i| format!("{prefix}::{}", camel(i)))
        .collect();
    arms.join(" | ")
}

fn literal(r: &Row, table: &str) -> String {
    let next = if r.next == "=" {
        "None".to_owned()
    } else {
        format!("Some(State::{})", camel(&r.next))
    };
    let act = match (r.act.as_str(), r.act.strip_prefix("refuse ")) {
        ("-", _) => "Do::Nothing".to_owned(),
        ("never", _) => "Do::Never".to_owned(),
        (_, Some(code)) => format!(
            "Do::Refuse(ErrorCode::{})",
            camel(&code.to_ascii_lowercase())
        ),
        (act, None) => format!("Do::Act(Action::{})", camel(act)),
    };
    let within = match (r.within.parse::<u32>(), r.within.strip_prefix("every ")) {
        (Ok(n), _) => format!("Within::Secs({n})"),
        (_, Some(n)) => format!("Within::Every({n})"),
        _ => "Within::Rest".to_owned(),
    };
    let clock = if r.owner == "=" {
        "None".to_owned()
    } else {
        let owner = camel(&r.owner);
        format!(
            "Some(Clock {{ owner: Owner::{owner}, within: {within}, or: {:?} }})",
            r.or
        )
    };
    format!(
        "const {id}: Row<{table}> = Row {{ id: {id:?}, next: {next}, act: {act}, clock: {clock} }};\n",
        id = r.id
    )
}

pub fn module(flow: &Flow) -> String {
    format!(
        "/// The {name} flow, from `flows/{name}.md`.\npub mod {name} {{\n{}}}\n",
        code(flow),
        name = flow.name
    )
}

fn code(flow: &Flow) -> String {
    let table = camel(&flow.name);
    let name = &flow.name;
    let states = variants(flow.states.iter().map(|s| (s.as_str(), format!("`{s}`"))));
    let facts = variants(
        flow.facts
            .iter()
            .map(|(f, s)| (f.as_str(), format!("`{f}`, from `{s}`"))),
    );
    let actions = variants(actions(flow).into_iter().map(|a| (a, format!("`{a}`"))));
    let rows: String = flow.rows.iter().map(|r| literal(r, &table)).collect();
    let arms: Vec<String> = flow
        .rows
        .iter()
        .map(|r| {
            let (states, facts) = (alt("State", &r.states), alt("Fact", &r.facts));
            format!("            ({states}, {facts}) => &{},\n", r.id)
        })
        .collect();
    let arms = arms.concat();
    let ids: Vec<&str> = flow.rows.iter().map(|r| r.id.as_str()).collect();
    format!(
        "{error}use super::{{Clock, Do, FlowName, Owner, Row, Table, Within}};

/// A state of the {name} flow; the wire value is its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = \"schema\", derive(schemars::JsonSchema))]
#[serde(rename_all = \"snake_case\")]
pub enum State {{
{states}}}

/// A fact the {name} flow takes, with its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fact {{
{facts}}}

/// An action a row of the {name} flow runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {{
{actions}}}

/// The {name} flow's table, from `flows/{name}.md`.
#[derive(Debug, Clone, Copy)]
pub struct {table};

{rows}
impl Table for {table} {{
    type State = State;
    type Fact = Fact;
    type Action = Action;
    const NAME: FlowName = FlowName::{table};
    const START: State = State::{start};
    const GAVE_UP: Fact = Fact::GaveUp;
    const ROWS: &'static [Row<Self>] = &[{ids}];
    const FACTS: &'static [Fact] = &[{all_facts}];

    fn row(state: State, fact: Fact) -> &'static Row<Self> {{
        match (state, fact) {{
{arms}        }}
    }}
}}
",
        start = camel(&flow.start),
        error = if flow.rows.iter().any(|r| r.act.starts_with("refuse ")) {
            "use crate::ErrorCode;\n"
        } else {
            ""
        },
        ids = ids.join(", "),
        all_facts = flow
            .facts
            .iter()
            .map(|(f, _)| format!("Fact::{}", camel(f)))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

fn statechart(flow: &Flow) -> String {
    let mut edges: Vec<((&str, &str), Vec<String>)> = Vec::new();
    for r in flow.rows.iter().filter(|r| r.next != "=") {
        for s in r.states.iter().filter(|s| **s != r.next) {
            let label = format!("{} ({})", r.facts.join(", "), r.id);
            match edges
                .iter_mut()
                .find(|(e, _)| *e == (s.as_str(), r.next.as_str()))
            {
                Some((_, labels)) => labels.push(label),
                None => edges.push(((s, &r.next), vec![label])),
            }
        }
    }
    let lines = edges
        .into_iter()
        .map(|((from, to), labels)| format!("    {from} --> {to}: {}\n", labels.join("; ")));
    format!(
        "\n## {}\n\n{}\n```mermaid\nstateDiagram-v2\n    direction LR\n    [*] --> {}\n{}```\n",
        flow.name,
        flow.text,
        flow.start,
        lines.collect::<Vec<_>>().concat()
    )
}

pub fn chart(flows: &[Flow]) -> String {
    let head = String::from(
        "# Flows\n\n<!-- Generated from crates/shed-proto/flows/ by `just generate`; do not edit. -->\n\n\
         Every shed flow's table, then its statechart: the rows that move. How to read a row: \
         [docs/architecture/state-table/tables.md](architecture/state-table/tables.md).\n",
    );
    head + &flows.iter().map(statechart).collect::<String>()
}
