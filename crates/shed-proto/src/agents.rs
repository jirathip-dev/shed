//! The agent records: what shed knows about each agent program, and the agents it has.

use std::collections::BTreeMap;

#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    BalanceProbe, CredentialsRecord, Price, ProxyRecord, StatusRead, TokenSetup, Tokens, UsageProbe,
};

/// What shed knows about one agent, as data: a built-in file, overridden key by key, or added, by a file of
/// the same name in `<config>/providers/` (`docs/acp/provider-records.md`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct Provider {
    /// The ACP agent id (`shed agents add <id>`).
    pub id: String,
    /// The agent program's own name.
    pub agent: String,
    /// Text in a line that says the login expired.
    pub auth_failures: Vec<String>,
    /// Text in a line that says a usage limit was hit.
    pub limit: Vec<String>,
    /// Markers in a limit line that the reset time follows.
    pub resets: Vec<String>,
    /// The command that logs in.
    pub login: Option<String>,
    /// The command that shows whether it is logged in.
    pub status: Option<String>,
    /// The command that logs out.
    pub logout: Option<String>,
    /// The variable that moves its config folder.
    pub config_dir_var: Option<String>,
    /// The variable that names its settings.
    pub settings_var: Option<String>,
    /// The command that installs the program.
    pub install: Option<String>,
    /// The variable that carries a login token.
    pub token_var: Option<String>,
    /// The models a start may name: id and name.
    pub models: Vec<[String; 2]>,
    /// The adapter's arguments that set the model, `{model}` standing for the pick.
    pub model_args: Vec<String>,
    /// The variable that sets the model, where the adapter takes one.
    pub model_env: Option<String>,
    /// Variables its logins, status checks and runs on an account get.
    pub env: BTreeMap<String, String>,
    /// The adapter's arguments that set the effort, `{effort}` standing for one of `efforts`.
    pub effort_args: Vec<String>,
    /// The effort levels it takes.
    pub efforts: Vec<String>,
    /// Where its prompt responses' `_meta` carries the turn's tokens.
    pub quota: Option<Quota>,
    /// How long its prompt cache lives on each login.
    pub cache: Option<Cache>,
    /// Which driver runs its lanes.
    pub driver: DriverKind,
    /// The stream driver's data: the agent's own JSON-lines mode.
    pub stream: Option<Stream>,
    /// The files and folders that make a config folder a setup: an account's folder holds none of them.
    pub setup_files: Vec<String>,
    /// Paths under `setup_files` the agent writes itself: a setup folder holding only these is no setup.
    pub setup_ignore: Vec<String>,
    /// A session's files under the config folder, `{session}` standing for its id.
    pub session_files: Vec<String>,
    /// The CLI's own config folder, `~/` for home: where `default`'s sessions are copied from, never to.
    pub default_folder: Option<String>,
    /// Variables, besides `cache.api_vars`, a run on an account never inherits.
    pub scrub: Vec<String>,
    /// How the status command's output reads.
    pub status_read: Option<StatusRead>,
    /// The usage probe.
    pub usage: Option<UsageProbe>,
    /// The balance probe.
    pub balance: Option<Box<BalanceProbe>>,
    /// The price per million tokens, for an API-key account's cost when the agent reports none.
    pub price: Option<Box<Price>>,
    /// The setup token.
    pub token: Option<TokenSetup>,
    /// The token proxy.
    pub proxy: Option<ProxyRecord>,
    /// Where the login lives, for the drift check.
    pub credentials: Option<CredentialsRecord>,
    /// What it needs inside a sandbox.
    pub sandbox: Option<Box<crate::SandboxNeeds>>,
}

/// The driver a provider's lanes run on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum DriverKind {
    /// Its ACP adapter.
    #[default]
    Acp,
    /// Its own JSON-lines mode, by the record's `stream` table, when a lane can be served whole.
    Stream,
}

/// How the stream driver starts an agent in its own JSON-lines mode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct Stream {
    /// The provider's line format: `providers/<wire>.rs`.
    pub wire: String,
    /// The agent's command line, before the arguments of one start.
    pub command: Vec<String>,
    /// Text that names the ACP adapter when it is the last argument of a caller's own command.
    pub adapter: Option<String>,
    /// The words an adapter is started with; `command` replaces them and the adapter.
    pub launchers: Vec<String>,
    /// The arguments that start a session under the id shed made (`{session}`).
    pub new: Vec<String>,
    /// The arguments that start the agent on a session it has (`{session}`).
    pub resume: Vec<String>,
    /// The arguments that set the model (`{model}`).
    pub model: Vec<String>,
    /// The arguments that set the effort (`{effort}`).
    pub effort: Vec<String>,
    /// The arguments that pass the MCP servers (`{file}`: a private file with the wire's text).
    pub mcp: Vec<String>,
    /// Each policy's arguments; a policy without an entry stays on ACP.
    pub policy: BTreeMap<String, Vec<String>>,
    /// Each `--meta` option's flag, by its dotted path; an option without an entry stays on ACP.
    pub meta: BTreeMap<String, String>,
    /// Each tool's ACP kind and the input key that is its title.
    pub tools: BTreeMap<String, [String; 2]>,
    /// The tool, and its input key, whose list is the plan.
    pub plan: Option<[String; 2]>,
}

/// The prompt cache's lifetime on a plan and on an API key, and the variables that mean an API key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Cache {
    /// Seconds on a subscription plan, the login without an API variable.
    pub plan: u64,
    /// Seconds on an API key.
    pub api: u64,
    /// Variables that, set and not empty in the agent's environment, mean an API key.
    pub api_vars: Vec<String>,
}

/// The keys of a turn's tokens in a prompt response's `_meta`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Quota {
    /// The keys from `_meta` down to the object of counters.
    pub path: Vec<String>,
    /// The input tokens' key.
    pub input: String,
    /// The cache reads' key.
    pub cache_read: String,
    /// The cache writes' key.
    pub cache_write: String,
    /// The output tokens' key.
    pub output: String,
}

/// What one start adds for a model and an effort: variables for the program and words after its command.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Picked {
    /// The variables, name and value.
    pub env: Vec<(String, String)>,
    /// The words.
    pub args: Vec<String>,
}

impl Provider {
    /// What one start on `driver` adds for a model and an effort; an effort it does not take is the error.
    ///
    /// # Errors
    /// An effort outside `efforts`, or a model or effort the driver has no words or variable for.
    pub fn pick(
        &self,
        driver: DriverKind,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Result<Picked, String> {
        let stream =
            (driver == DriverKind::Stream).then(|| self.stream.clone().unwrap_or_default());
        let (model_args, effort_args) = match &stream {
            Some(s) => (&s.model, &s.effort),
            None => (&self.model_args, &self.effort_args),
        };
        let with = |args: &[String], key: &str, value: &str| {
            args.iter().map(|a| a.replace(key, value)).collect()
        };
        let mut picked = Picked::default();
        if let Some(model) = model {
            match (&self.model_env, stream.is_none()) {
                (Some(var), true) => picked.env.push((var.clone(), model.to_owned())),
                _ if model_args.is_empty() => {
                    return Err(format!("{} takes no model on this driver", self.id));
                }
                _ => picked.args = with(model_args, "{model}", model),
            }
        }
        if let Some(effort) = effort {
            if !self.efforts.iter().any(|e| e == effort) || effort_args.is_empty() {
                return Err(format!(
                    "{} takes no effort {effort} on this driver",
                    self.id
                ));
            }
            picked.args.extend(with(effort_args, "{effort}", effort));
        }
        Ok(picked)
    }

    /// The first line of `text` that says a usage limit was hit, and the reset time after a marker in it.
    #[must_use]
    pub fn limit_hit<'t>(&self, text: &'t str) -> Option<(&'t str, Option<String>)> {
        let line = text
            .lines()
            .find(|l| self.limit.iter().any(|s| l.contains(s)))?;
        let at = self.resets.iter().find_map(|r| line.split_once(r));
        Some((
            line,
            at.map(|(_, at)| at.trim().trim_end_matches('.').to_owned()),
        ))
    }

    /// The turn's tokens in a prompt response's `meta`, or `None` when they are absent or not counts.
    #[must_use]
    pub fn tokens(&self, meta: &serde_json::Map<String, Value>) -> Option<Tokens> {
        let quota = self.quota.as_ref()?;
        let (first, rest) = quota.path.split_first()?;
        let counts = rest.iter().try_fold(meta.get(first)?, |v, k| v.get(k))?;
        let count = |key: &String| counts.get(key).and_then(Value::as_u64);
        Some(Tokens {
            input: count(&quota.input)?,
            cache_read: count(&quota.cache_read)?,
            cache_write: count(&quota.cache_write)?,
            output: count(&quota.output)?,
        })
    }

    /// The prompt cache's likely lifetime in seconds, by whether `set` says one of the API variables is set.
    #[must_use]
    pub fn cache_lifetime(&self, set: impl Fn(&str) -> bool) -> Option<u64> {
        let cache = self.cache.as_ref()?;
        let api = cache.api_vars.iter().any(|v| set(v));
        Some(if api { cache.api } else { cache.plan })
    }

    /// The first line of `text` that says the login expired.
    #[must_use]
    pub fn auth_failure<'t>(&self, text: &'t str) -> Option<&'t str> {
        text.lines()
            .find(|l| self.auth_failures.iter().any(|s| l.contains(s)))
    }
}

/// One installed ACP agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Agent {
    /// The registry id.
    pub id: String,
    /// The pinned version.
    pub version: String,
    /// The distribution it came from: `binary`, `npx` (installed once with npm, run by `node`) or `uvx`.
    pub distribution: String,
    /// The ACP adapter's command line (`env K=V …` first when the registry sets variables).
    pub cmd: Vec<String>,
}

/// One `agents` entry: an agent shed has a provider record for, installed, or both.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct AgentInfo {
    /// The ACP agent id.
    pub id: String,
    /// Where the record came from: `built-in` or the user file's path.
    pub source: Option<String>,
    /// The provider record.
    pub provider: Option<Provider>,
    /// The installed adapter.
    pub installed: Option<Agent>,
    /// The medians of its last sessions' timings, when any session got a first token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medians: Option<Medians>,
}

/// An agent's medians over its last `$SHED_MEDIAN_OF` (default 20) ACP sessions that got a first token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Medians {
    /// How many sessions they are over.
    pub sessions: u64,
    /// The median of [`crate::AcpSession::ready_ms`].
    pub ready_ms: u64,
    /// The median of [`crate::AcpSession::first_token_ms`].
    pub first_token_ms: u64,
}

/// `agents.match` params: an agent's id and text to apply its rules to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct MatchParams {
    /// The ACP agent id.
    pub id: String,
    /// The text, e.g. a pane's screen.
    pub text: String,
}

/// `agents.match` result: the lines the agent's rules find.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Matched {
    /// The first line that says a usage limit was hit (the state `rate-limited`).
    pub limit: Option<String>,
    /// The reset time that line gives.
    pub resets: Option<String>,
    /// The first line that says the login expired.
    pub auth: Option<String>,
}
