//! The agent records in three layers, each a table of keys over the record of the same agent below it: the
//! built-in `providers/<agent>.toml`, the host's `<config>/providers/<agent>.toml`, and an app's own rows. A
//! record with no `id` takes its file's name.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use shed_proto::{AgentInfo, Provider};
use toml::Table;

const BUILT_IN: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/providers.rs"));

/// One record and where its top layer came from: `built-in`, a file's path, or `app`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The record.
    pub provider: Provider,
    /// Where it came from.
    pub source: String,
}

/// The records, by agent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    entries: BTreeMap<String, Entry>,
}

impl Catalog {
    /// The records compiled in; no server, no file.
    #[must_use]
    pub fn built_in() -> Self {
        let mut catalog = Self::default();
        for (agent, text) in BUILT_IN {
            catalog.put(agent, text, "built-in").ok();
        }
        catalog
    }

    /// The built-in records under the host's files in `dir`, and an error for each file that is unreadable,
    /// not TOML, or not a record; those are skipped.
    #[must_use]
    pub fn load(dir: &Path) -> (Self, Vec<String>) {
        let mut catalog = Self::built_in();
        let mut bad = Vec::new();
        let mut files: Vec<_> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        files.sort();
        for path in files
            .iter()
            .filter(|p| p.extension() == Some("toml".as_ref()))
        {
            let agent = path.file_stem().unwrap_or_default().to_string_lossy();
            let source = path.display().to_string();
            let read = fs::read_to_string(path).map_err(|e| e.to_string());
            if let Err(e) = read.and_then(|text| catalog.put(&agent, &text, &source)) {
                bad.push(format!("{source}: {e}"));
            }
        }
        (catalog, bad)
    }

    /// The records a server holds, from its answer to `agents`.
    #[must_use]
    pub fn served(agents: &[AgentInfo]) -> Self {
        let ids: BTreeSet<&str> = agents.iter().map(|a| a.id.as_str()).collect();
        let entry = |a: &AgentInfo| {
            let provider = a.provider.clone()?;
            let agent = Some(provider.agent.clone())
                .filter(|n| !n.is_empty() && (*n == a.id || !ids.contains(n.as_str())));
            let source = a.source.clone().unwrap_or_default();
            Some((
                agent.unwrap_or_else(|| a.id.clone()),
                Entry { provider, source },
            ))
        };
        Self {
            entries: agents.iter().filter_map(entry).collect(),
        }
    }

    /// An app's row: its keys over `agent`'s record, or a new record.
    ///
    /// # Errors
    /// A row that is not TOML or does not make a record, an unknown key included, named by `agent`.
    pub fn over(mut self, agent: &str, toml: &str) -> Result<Self, String> {
        self.put(agent, toml, "app")
            .map_err(|e| format!("{agent}: {e}"))?;
        Ok(self)
    }

    /// The record of ACP agent `id`.
    #[must_use]
    pub fn by_id(&self, id: &str) -> Option<&Provider> {
        self.iter().map(|e| &e.provider).find(|p| p.id == id)
    }

    /// The record of the program named `agent`.
    #[must_use]
    pub fn by_agent(&self, agent: &str) -> Option<&Provider> {
        self.iter().map(|e| &e.provider).find(|p| p.agent == agent)
    }

    /// Every record, by agent.
    pub fn iter(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    fn put(&mut self, agent: &str, text: &str, source: &str) -> Result<(), String> {
        let row: Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
        let below = self
            .entries
            .get(agent)
            .map(|e| Table::try_from(&e.provider));
        let mut table = below
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        table.extend(row);
        if table
            .get("id")
            .and_then(toml::Value::as_str)
            .is_none_or(str::is_empty)
        {
            table.insert("id".to_owned(), agent.into());
        }
        let provider = table
            .try_into()
            .map_err(|e: toml::de::Error| e.to_string())?;
        let source = source.to_owned();
        self.entries
            .insert(agent.to_owned(), Entry { provider, source });
        Ok(())
    }
}
