//! Stamps the build with its git commit (`SHED_BUILD`), and writes `Method` and `ErrorCode` from `methods/*.md` and `errors/*.md`, one file a row.

#[path = "../../build/collect.rs"]
mod collect;

use std::collections::BTreeSet;
use std::process::Command;

use collect::Record;

const METHOD_KEYS: [&str; 8] = [
    "wire", "params", "result", "stream", "schema", "on", "input", "hello",
];

const IN_SCOPE: [&str; 5] = ["BTreeMap", "Empty", "String", "Value", "Vec"];

fn imports(records: &[Record], keys: &[&str]) -> String {
    let mut names = BTreeSet::new();
    for value in records
        .iter()
        .flat_map(|r| keys.iter().filter_map(|k| r.get(k)))
    {
        let words = value.split(|c: char| !c.is_alphanumeric() && c != '_');
        names.extend(words.filter(|w| w.starts_with(char::is_uppercase) && !IN_SCOPE.contains(w)));
    }
    format!(
        "use crate::{{{}}};",
        names.into_iter().collect::<Vec<_>>().join(", ")
    )
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    out.status.success().then(|| text.trim().to_owned())
}

fn stamp() {
    let mut watched = vec!["HEAD".to_owned(), "packed-refs".to_owned()];
    watched.extend(git(&["symbolic-ref", "-q", "HEAD"]));
    for path in watched {
        let file = git(&["rev-parse", "--path-format=absolute", "--git-path", &path]);
        if let Some(file) = file.filter(|f| std::path::Path::new(f).exists()) {
            println!("cargo:rerun-if-changed={file}");
        }
    }
    let sha = git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=SHED_BUILD={sha}");
}

fn arms(records: &[Record], arm: impl Fn(&Record) -> Option<String>) -> String {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for r in records {
        let Some(body) = arm(r) else { continue };
        let variant = format!("Self::{}", r.variant);
        match groups.iter_mut().find(|(b, _)| *b == body) {
            Some((_, variants)) => variants.push(variant),
            None => groups.push((body, vec![variant])),
        }
    }
    groups
        .into_iter()
        .map(|(body, variants)| format!("{} => {body},\n", variants.join(" | ")))
        .collect::<Vec<_>>()
        .concat()
}

fn variants(records: &[Record], doc: impl Fn(&Record) -> String) -> String {
    records
        .iter()
        .map(|r| format!("#[doc = {:?}]\n{},\n", doc(r), r.variant))
        .collect::<Vec<_>>()
        .concat()
}

fn list(records: &[Record], keep: impl Fn(&Record) -> bool) -> String {
    let names: Vec<String> = records
        .iter()
        .filter(|r| keep(r))
        .map(|r| format!("Self::{}", r.variant))
        .collect();
    names.join(" | ")
}

fn one_of(r: &Record, key: &str, values: &[&str]) -> Result<(), String> {
    match r.get(key) {
        Some(v) if !values.contains(&v) => {
            Err(format!("{}: `{key}: {v}` is not one of {values:?}", r.file))
        }
        _ => Ok(()),
    }
}

fn methods() -> Result<String, String> {
    let mut records = collect::records("methods", &METHOD_KEYS, &["wire"])?;
    for r in &records {
        one_of(r, "on", &["machine", "name"])?;
        one_of(r, "input", &["yes"])?;
        one_of(r, "hello", &["no"])?;
        if r.prose.is_empty() {
            return Err(format!("{}: no prose after the keys", r.file));
        }
    }
    records.sort_by(|a, b| a.get("wire").cmp(&b.get("wire")));
    let wire = |r: &Record| r.get("wire").unwrap_or_default().to_owned();
    let ty = |r: &Record, key| r.get(key).unwrap_or("Empty").to_owned();
    let route = |r: &Record| match r.get("on") {
        Some("machine") => "Some(Route::Machine)",
        Some(_) => "Some(Route::Name)",
        None => "None",
    };
    let schemas = |r: &Record| {
        let pair = format!("pair::<{}, {}>(g)", ty(r, "params"), ty(r, "result"));
        Some(
            r.get("schema")
                .map_or(pair.clone(), |f| format!("{f}({pair})")),
        )
    };
    let stream = |r: &Record| {
        r.get("stream")
            .map(|t| format!("Some(g.subschema_for::<{t}>())"))
    };
    Ok(format!(
        "/// The methods, with their parameter and result types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {{
{variants}}}

impl Method {{
    /// Every method, in wire-name order.
    pub const ALL: [Self; {n}] = [{all}];

    /// The wire name.
    #[must_use]
    pub fn name(self) -> &'static str {{
        match self {{
{names}        }}
    }}

    /// What the method does.
    #[must_use]
    pub fn summary(self) -> &'static str {{
        match self {{
{summaries}        }}
    }}

    /// The field that names the machine a held link carries the request to, if any.
    #[must_use]
    pub fn route(self) -> Option<Route> {{
        match self {{
{routes}        }}
    }}

    /// Whether a connection that marked the terminal read-only is refused it.
    #[must_use]
    pub fn input(self) -> bool {{
        matches!(self, {inputs})
    }}

    /// Whether it is refused before the handshake.
    #[must_use]
    pub fn needs_hello(self) -> bool {{
        !matches!(self, {no_hello})
    }}
}}

#[cfg(feature = \"schema\")]
impl Method {{
    fn schemas(self, g: &mut SchemaGenerator) -> [Schema; 2] {{
        {schema_types}
        match self {{
{schemas}        }}
    }}

    fn stream(self, g: &mut SchemaGenerator) -> Option<Schema> {{
        {stream_types}
        match self {{
{streams}            _ => None,
        }}
    }}
}}
",
        variants = variants(&records, |r| format!(
            "`{}`: `{}` → `{}`.",
            wire(r),
            ty(r, "params"),
            ty(r, "result")
        )),
        n = records.len(),
        all = list(&records, |_| true).replace(" |", ","),
        names = arms(&records, |r| Some(format!("{:?}", wire(r)))),
        summaries = arms(&records, |r| Some(format!("{:?}", r.prose.join(" ")))),
        routes = arms(&records, |r| Some(route(r).to_owned())),
        inputs = list(&records, |r| r.get("input").is_some()),
        no_hello = list(&records, |r| r.get("hello").is_some()),
        schemas = arms(&records, schemas),
        streams = arms(&records, stream),
        schema_types = imports(&records, &["params", "result"]),
        stream_types = imports(&records, &["stream"]),
    ))
}

fn errors() -> Result<String, String> {
    let records = collect::records("errors", &["name", "code"], &["name", "code"])?;
    let mut coded = Vec::new();
    for r in records {
        let code = r.get("code").unwrap_or_default();
        let code: i64 = code
            .parse()
            .map_err(|e| format!("{}: `code: {code}`: {e}", r.file))?;
        coded.push((code, r));
    }
    coded.sort_by_key(|(code, _)| std::cmp::Reverse(*code));
    let records: Vec<Record> = coded.into_iter().map(|(_, r)| r).collect();
    Ok(format!(
        "/// The error codes: shed's own from -32000 down, then JSON-RPC's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {{
{variants}}}

impl ErrorCode {{
    /// Every code, in code order from -32000 down.
    pub const ALL: [Self; {n}] = [{all}];

    /// The numeric JSON-RPC code.
    #[must_use]
    pub fn code(self) -> i64 {{
        match self {{
{codes}        }}
    }}

    /// The name that starts the error message.
    #[must_use]
    pub fn name(self) -> &'static str {{
        match self {{
{names}        }}
    }}
}}
",
        variants = variants(&records, |r| r.prose.join(" ")),
        n = records.len(),
        all = list(&records, |_| true).replace(" |", ","),
        codes = arms(&records, |r| r.get("code").map(str::to_owned)),
        names = arms(&records, |r| r.get("name").map(|n| format!("{n:?}"))),
    ))
}

fn main() -> Result<(), String> {
    stamp();
    collect::write("methods.rs", &methods()?).map_err(|e| e.to_string())?;
    collect::write("errors.rs", &errors()?).map_err(|e| e.to_string())
}
