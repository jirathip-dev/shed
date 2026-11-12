//! `shed setup`: the hooks that make an installed agent report its state through `shed state`.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::providers::{Hooks, hook_tables};

const MARK: &str = "|| shed state ";

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

fn edit(home: &Path, h: &Hooks, remove: bool, dry: bool) -> io::Result<HookFile> {
    let file = home.join(h.file);
    let before = fs::read_to_string(&file).or_else(|e| match e.kind() {
        ErrorKind::NotFound => Ok(String::new()),
        _ => Err(e),
    })?;
    let text = Some(before.trim()).filter(|t| !t.is_empty());
    let mut root: Value = serde_json::from_str(text.unwrap_or("{}"))?;
    let original = root.clone();
    let bad = || io::Error::other(format!("{}: not JSON objects", file.display()));
    let obj = root.as_object_mut().ok_or_else(bad)?;
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    let events = hooks.as_object_mut().ok_or_else(bad)?;
    for (event, state) in h.events {
        let list = events.entry(*event).or_insert_with(|| json!([]));
        let groups = list.as_array_mut().ok_or_else(bad)?;
        let ours = |g: &Value| g.to_string().contains(MARK);
        if remove {
            groups.retain(|g| !ours(g));
        } else if !groups.iter().any(ours) {
            let hook =
                json!({"type": "command", "command": format!("[ -z \"$SHED\" ] {MARK}{state}")});
            groups.push(json!({"hooks": [hook]}));
        }
        if groups.is_empty() {
            events.shift_remove(*event);
        }
    }
    if events.is_empty() {
        obj.shift_remove("hooks");
    }
    let after = serde_json::to_string_pretty(&root)? + "\n";
    let empty = root.as_object().is_some_and(serde_json::Map::is_empty);
    match (dry || root == original, empty) {
        (true, _) => {}
        (false, true) => fs::remove_file(&file)?,
        (false, false) => fs::write(&file, &after)?,
    }
    let after = (root != original).then_some(after);
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

    const OTHERS: &str = r#"{
  "model": "x",
  "hooks": {
    "Stop": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "say done"
          }
        ]
      }
    ],
    "Other": []
  },
  "alpha": true
}
"#;

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

        fs::write(&file, OTHERS).expect("config");
        let preview = run(&home, false, true).expect("a preview");
        assert_eq!(
            fs::read_to_string(&file).expect("config"),
            OTHERS,
            "dry run writes nothing"
        );
        assert_eq!(run(&home, false, false), Some(preview.clone()));
        assert!(preview.contains("say done") && preview.contains(r#""alpha": true"#));
        assert!(
            preview.find("say done") < preview.find("state done"),
            "{preview}"
        );
        assert_eq!(
            run(&home, false, false),
            None,
            "a second run changes nothing"
        );
        assert_eq!(fs::read_to_string(&file).expect("config"), preview);

        run(&home, true, false);
        assert_eq!(fs::read_to_string(&file).expect("config"), OTHERS);
    }
}
