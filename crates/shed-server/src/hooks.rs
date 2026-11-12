//! `shed setup`: the hooks that make an installed agent report its state through `shed state`.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use serde_json::value::RawValue;
use serde_json::{Value, json};

use crate::providers::{Hooks, hook_tables};

const EMPTY: &str = "{}\n";

/// One agent's config file and, when `shed setup` changes it, its new text.
pub struct HookFile {
    /// The agent's name, as `shed setup <agent>` takes it.
    pub agent: &'static str,
    /// Its config file.
    pub file: PathBuf,
    /// The new text (`None`: unchanged); a file left an empty object is deleted.
    pub after: Option<String>,
}

/// Adds (or with `remove` takes out) the `shed state` hooks of the installed `agents` (all when
/// empty) in their config files under `home`, keeping every other entry; `dry` writes nothing.
///
/// # Errors
/// An agent with no hooks, a config file that is not JSON objects, or I/O.
pub fn setup(home: &Path, agents: &[String], remove: bool, dry: bool) -> io::Result<Vec<HookFile>> {
    if let Some(a) = agents.iter().find(|a| hook_tables().all(|h| h.agent != *a)) {
        return Err(io::Error::other(format!("shed has no hooks for {a}")));
    }
    let wanted = |h: &&Hooks| agents.is_empty() || agents.iter().any(|a| a == h.agent);
    let installed = |h: &&Hooks| home.join(h.file).parent().is_some_and(Path::is_dir);
    let chosen = hook_tables().filter(wanted).filter(installed);
    chosen.map(|h| edit(home, h, remove, dry)).collect()
}

type Object<'a> = HashMap<String, &'a RawValue>;

fn group(state: &str) -> Value {
    let command = format!("[ -z \"$SHED\" ] || shed state {state}");
    json!({"hooks": [{"type": "command", "command": command}]})
}

fn member(event: &str, state: &str) -> String {
    format!("{}:[ {}]", json!(event), group(state))
}

fn whole(h: &Hooks) -> String {
    let members: Vec<String> = h.events.iter().map(|(e, s)| member(e, s)).collect();
    format!("\"hooks\":{{ {}}}", members.join(","))
}

fn inside(text: &str, raw: &RawValue) -> usize {
    raw.get()
        .as_ptr()
        .addr()
        .saturating_sub(text.as_ptr().addr())
        + 1
}

fn add(text: &str, h: &Hooks) -> serde_json::Result<String> {
    let root: Object = serde_json::from_str(text)?;
    let mut at: BTreeMap<usize, (Vec<String>, bool)> = BTreeMap::new();
    if let Some(hooks) = root.get("hooks") {
        let events: Object = serde_json::from_str(hooks.get())?;
        for (event, state) in h.events {
            let (raw, piece, len) = match events.get(*event) {
                Some(list) => {
                    let groups: Vec<Value> = serde_json::from_str(list.get())?;
                    if groups.contains(&group(state)) {
                        continue;
                    }
                    (*list, group(state).to_string(), groups.len())
                }
                None => (*hooks, member(event, state), events.len()),
            };
            let slot = at
                .entry(inside(text, raw))
                .or_insert((Vec::new(), len == 0));
            slot.0.push(piece);
        }
    } else {
        let raw: &RawValue = serde_json::from_str(text)?;
        at.insert(inside(text, raw), (vec![whole(h)], root.is_empty()));
    }
    let mut out = text.to_owned();
    for (pos, (pieces, empty)) in at.into_iter().rev() {
        out.insert_str(pos, &(pieces.join(",") + if empty { "" } else { "," }));
    }
    Ok(out)
}

fn strip(text: &str, h: &Hooks) -> String {
    let mut out = text.to_owned();
    let members = h.events.iter().map(|(e, s)| member(e, s));
    let groups = h.events.iter().map(|(_, s)| group(s).to_string());
    for piece in std::iter::once(whole(h)).chain(members).chain(groups) {
        while let Some(i) = out.find(&piece) {
            let end = i + piece.len();
            let comma = out.get(end..).is_some_and(|r| r.starts_with(','));
            out.replace_range(i..end + usize::from(comma), "");
        }
    }
    out
}

fn edit(home: &Path, h: &Hooks, remove: bool, dry: bool) -> io::Result<HookFile> {
    let file = home.join(h.file);
    let before = fs::read_to_string(&file).or_else(|e| match e.kind() {
        ErrorKind::NotFound => Ok(EMPTY.to_owned()),
        _ => Err(e),
    })?;
    let bad = |e| io::Error::other(format!("{}: not JSON objects: {e}", file.display()));
    let after = if remove {
        strip(&before, h)
    } else {
        add(&before, h).map_err(bad)?
    };
    let after = (after != before).then_some(after);
    match (&after, dry) {
        (Some(t), false) if t == EMPTY => fs::remove_file(&file)?,
        (Some(t), false) => fs::write(&file, t)?,
        _ => {}
    }
    Ok(HookFile {
        agent: h.agent,
        file,
        after,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::setup;

    const OTHERS: &[&str] = &[
        "{\"model\":\"x\",\n    \"permissions\": {\"allow\": [\"Bash(ls:*)\"]}, \"n\": 1.0}",
        "{ \"hooks\" : {\"Stop\":[ {\"matcher\": \"\", \"hooks\": [{\"type\":\"command\",\n\t\"command\": \"say done\"}]} ], \"Other\": []},\n  \"alpha\": true }\n",
        "{\"hooks\":{\"Prompt\":[],\"Stop\": [ ]},\"n\":2}",
        "  {\"hooks\": {}}\n",
    ];

    fn run(home: &Path, remove: bool, dry_run: bool) -> Option<String> {
        let fake = ["fake".to_owned()];
        let files = setup(home, &fake, remove, dry_run).expect("setup");
        let [file] = files.as_slice() else {
            panic!("{} files", files.len())
        };
        file.after.clone()
    }

    #[test]
    fn setup_adds_its_hooks_once_and_remove_restores_the_file_byte_for_byte() {
        let home = crate::temp_dir("setup-hooks");
        let file = home.join(".fake/settings.json");
        assert!(setup(&home, &["nope".to_owned()], false, false).is_err());
        assert!(
            setup(&home, &["fake".to_owned()], false, false)
                .expect("none")
                .is_empty()
        );
        fs::create_dir(home.join(".fake")).expect("agent dir");
        let found = setup(&home, &[], false, true).expect("dry run");
        assert!(found.iter().any(|f| f.agent == "fake"));

        let added = run(&home, false, false).expect("added");
        assert!(added.contains(r#"|| shed state working""#), "{added}");
        assert!(added.contains(r#"|| shed state done""#), "{added}");
        assert_eq!(fs::read_to_string(&file).expect("written"), added);
        assert_eq!(run(&home, false, true), None, "installed: nothing to add");
        run(&home, true, false);
        assert!(!file.exists(), "an absent config is absent again");

        for others in OTHERS {
            fs::write(&file, others).expect("config");
            let preview = run(&home, false, true).expect("a preview");
            assert_eq!(
                fs::read_to_string(&file).expect("config"),
                *others,
                "dry run"
            );
            assert_eq!(run(&home, false, false), Some(preview.clone()));
            let value: serde_json::Value = serde_json::from_str(&preview).expect(&preview);
            let hooks = &value["hooks"];
            assert!(
                hooks["Prompt"].to_string().contains("shed state working"),
                "{preview}"
            );
            assert!(
                hooks["Stop"].to_string().contains("shed state done"),
                "{preview}"
            );
            assert!(!others.contains("say done") || preview.contains("say done"));
            assert_eq!(
                run(&home, false, false),
                None,
                "a second run changes nothing"
            );
            assert_eq!(fs::read_to_string(&file).expect("config"), preview);
            run(&home, true, false);
            assert_eq!(fs::read_to_string(&file).expect("config"), *others);
        }
    }
}
