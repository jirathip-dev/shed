//! The accounts' keys of a provider record, and what its status command and usage probe say.

use std::collections::BTreeMap;
use std::path::PathBuf;

#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Provider;

const DAY: u64 = 86_400;

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// How the status command's output reads; without it, exit 0 is signed in and the output is the status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct StatusRead {
    /// The output's format.
    pub format: StatusFormat,
    /// The key whose `true` means signed in.
    pub signed_in: String,
    /// The keys that name who it is signed in as, the first one set wins.
    #[serde(default)]
    pub who: Vec<String>,
    /// The key that names how it signed in.
    #[serde(default)]
    pub how: Option<String>,
}

/// A status command's output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum StatusFormat {
    /// One JSON object.
    Json,
}

/// The usage probe: the command that prints an account's usage, and how its lines read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct UsageProbe {
    /// The command.
    pub command: String,
    /// Variables it runs with.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Variables it runs without.
    #[serde(default)]
    pub unset: Vec<String>,
    /// Seconds a reading stands.
    pub fresh: u64,
    /// Seconds a limit mark stands when no reading gives a reset time.
    pub hold: u64,
    /// The windows it reads.
    pub windows: Vec<WindowRead>,
}

/// How one usage window's line reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct WindowRead {
    /// The window's name.
    pub id: String,
    /// The text its line starts with.
    pub line: String,
    /// The marker the percentage used comes before.
    pub used: String,
    /// The marker the reset time comes after.
    pub resets: String,
}

/// The setup token: the command that makes one, how it starts and how long it lasts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TokenSetup {
    /// The command that prints a new token.
    pub command: String,
    /// The text the token on the screen starts with.
    pub prefix: String,
    /// The days it lasts.
    pub days: u64,
}

/// The token proxy: where a run on another machine's calls go, and how they are signed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ProxyRecord {
    /// The provider's API.
    pub upstream: String,
    /// The path prefixes it signs; a request outside them is refused.
    pub paths: Vec<String>,
    /// The variable that points the agent at the proxy.
    pub base_url_var: String,
    /// The variable that carries the run's key.
    pub key_var: String,
    /// The header that carries the token, `{token}` standing for it.
    pub auth: String,
    /// Values added to a header's list when the request lacks them.
    #[serde(default)]
    pub append: BTreeMap<String, String>,
}

/// What a status command says of an account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Signed in.
    pub signed_in: bool,
    /// The status on one line, not yet redacted.
    pub line: String,
    /// The email or organisation it is signed in as.
    pub who: Option<String>,
}

/// A usage reading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Usage {
    /// Its windows.
    pub windows: Vec<Window>,
    /// When it was read.
    pub read_at: u64,
}

/// One usage window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Window {
    /// Its name.
    pub id: String,
    /// The percentage used.
    pub used_pct: u32,
    /// When it resets.
    pub resets_at: Option<u64>,
}

/// An account: a provider's login in a config folder shed keeps, and what shed last read of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Account {
    /// The record's `agent` name.
    pub provider: String,
    /// `[a-z0-9-]+`, or `default`: the CLI's own default folder.
    pub name: String,
    /// Its config folder; none for `default`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<PathBuf>,
    /// An adopted folder, linked in: `account.rm` removes the link, never the folder.
    #[serde(default)]
    pub adopted: bool,
    /// Signed out, ready or limited.
    pub state: AccountState,
    /// The email or organisation the status command reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Option<String>,
    /// The status on one line, redacted.
    #[serde(default)]
    pub line: String,
    /// When the status command last ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<u64>,
    /// The last usage reading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Its setup token's dates, never the token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<TokenInfo>,
    /// The record's `setup_files` found in its folder; not empty: no run.
    #[serde(default)]
    pub setup: Vec<String>,
    /// Its login terminal, while one runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
}

/// An account's state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum AccountState {
    /// Neither signed in nor holding a token.
    #[default]
    SignedOut,
    /// The status command says signed in, or it has a token.
    Ready,
    /// It hit a limit.
    Limited,
}

/// A setup token's dates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct TokenInfo {
    /// When it was saved.
    pub saved_at: u64,
    /// When it lapses, by the record's `token.days`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    /// The upstream refused it.
    #[serde(default)]
    pub rejected: bool,
}

/// `account.add` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct AccountAdd {
    /// The record's `agent` name.
    pub provider: String,
    /// `[a-z0-9-]+`.
    pub name: String,
    /// Adopt this existing folder in place, by link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<PathBuf>,
    /// A setup token file on this host (0600), copied in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_file: Option<PathBuf>,
}

/// `account.status`, `account.login` and `account.rm` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct AccountName {
    /// The record's `agent` name.
    pub provider: String,
    /// The account.
    pub name: String,
}

/// `account.ls` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct AccountsParams {
    /// Only this provider's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// `account.login`'s result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Login {
    /// The redacted terminal the login runs in.
    pub terminal: String,
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Provider {
    /// What the status command said, by its exit, its stdout and its stderr.
    #[must_use]
    pub fn read_status(&self, ok: bool, stdout: &str, stderr: &str) -> Status {
        let out = if stdout.trim().is_empty() {
            stderr
        } else {
            stdout
        };
        let plain = || Status {
            signed_in: ok,
            line: one_line(out),
            who: None,
        };
        let Some(read) = &self.status_read else {
            return plain();
        };
        let Ok(Value::Object(v)) = serde_json::from_str(stdout) else {
            return plain();
        };
        let text = |k: &str| v.get(k).and_then(Value::as_str);
        let who = read.who.iter().find_map(|k| text(k));
        let how = read.how.as_deref().and_then(text);
        let signed_in = v.get(&read.signed_in).and_then(Value::as_bool);
        let line = match (signed_in, how, who) {
            (Some(true), Some(how), Some(who)) => format!("signed in · {how} ({who})"),
            (Some(true), Some(how), None) => format!("signed in · {how}"),
            (Some(true), None, _) => "signed in".to_owned(),
            (Some(false), ..) => "signed out".to_owned(),
            (None, ..) => "status unknown".to_owned(),
        };
        Status {
            signed_in: signed_in.unwrap_or(ok),
            line: one_line(&line),
            who: who.map(one_line),
        }
    }

    /// The usage probe's reading at `now`; none without a probe or when no window reads.
    #[must_use]
    pub fn read_usage(&self, stdout: &str, now: u64) -> Option<Usage> {
        let probe = self.usage.as_ref()?;
        let window = |w: &WindowRead| {
            let rest = (stdout.lines()).find_map(|l| l.trim().strip_prefix(w.line.as_str()))?;
            let (used, after) = rest.split_once(w.used.as_str())?;
            let resets = after.split_once(w.resets.as_str());
            Some(Window {
                id: w.id.clone(),
                used_pct: used.split_whitespace().last()?.parse().ok()?,
                resets_at: resets.and_then(|(_, at)| when(at, now)),
            })
        };
        let windows: Vec<_> = probe.windows.iter().filter_map(window).collect();
        (!windows.is_empty()).then_some(Usage {
            windows,
            read_at: now,
        })
    }
}

/// The time `text` names, seen at `now`, all in UTC: unix seconds, `3pm` or `3:30pm` (the next such time),
/// `Nov 16 at 2am` (the nearest such date) or `in 2h 5m`; a trailing `(UTC)` is allowed, another zone is not.
#[must_use]
pub fn when(text: &str, now: u64) -> Option<u64> {
    let text = text.trim().trim_end_matches('.').to_ascii_lowercase();
    let text = text
        .strip_suffix("(utc)")
        .map_or(text.as_str(), str::trim_end);
    if let Ok(secs) = text.parse() {
        return Some(secs);
    }
    if let Some(span) = text.strip_prefix("in ") {
        return after(span, now);
    }
    let (date, time) = (text.rsplit_once(" at ")).map_or((None, text), |(d, t)| (Some(d), t));
    let secs = clock(time)?;
    let Some(date) = date else {
        let t = now - now % DAY + secs;
        return Some(if t < now { t + DAY } else { t });
    };
    let (month, day) = date.split_once(' ')?;
    let month = u64::try_from(MONTHS.iter().position(|m| *m == month)?).ok()? + 1;
    let day = day.trim().parse().ok().filter(|d| (1..=31).contains(d))?;
    let year = 1970 + now / 31_556_952;
    (year - 1..=year + 1)
        .filter_map(|y| Some(civil_days(y, month, day)? * DAY + secs))
        .find(|t| t + DAY >= now)
}

fn clock(time: &str) -> Option<u64> {
    let (time, half) = match time.strip_suffix("pm") {
        Some(t) => (t, 12),
        None => (time.strip_suffix("am")?, 0),
    };
    let (h, m) = time.split_once(':').unwrap_or((time, "0"));
    let (h, m): (u64, u64) = (h.parse().ok()?, m.parse().ok()?);
    (h <= 12 && m < 60).then_some((h % 12 + half) * 3600 + m * 60)
}

fn after(span: &str, now: u64) -> Option<u64> {
    span.split_whitespace().try_fold(now, |t, part| {
        let unit = part.trim_start_matches(|c: char| c.is_ascii_digit());
        let n: u64 = part.strip_suffix(unit)?.parse().ok()?;
        let secs = match unit {
            "d" => DAY,
            "h" => 3600,
            "m" => 60,
            "s" => 1,
            _ => return None,
        };
        t.checked_add(n.checked_mul(secs)?)
    })
}

fn civil_days(y: u64, m: u64, d: u64) -> Option<u64> {
    let y = if m <= 2 { y - 1 } else { y };
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = (y % 400) * 365 + (y % 400) / 4 - (y % 400) / 100 + doy;
    ((y / 400) * 146_097 + doe).checked_sub(719_468)
}

#[cfg(test)]
mod tests {
    use super::{DAY, civil_days, when};

    fn day(y: u64, m: u64, d: u64) -> u64 {
        civil_days(y, m, d).unwrap_or(u64::MAX) * DAY
    }

    #[test]
    fn when_reads_its_four_forms_in_utc() {
        let now = day(2026, 11, 10) + 9 * 3600;
        assert_eq!(day(1970, 1, 1), 0);
        assert_eq!(when("1763000000", now), Some(1_763_000_000));
        assert_eq!(
            when("Nov 16 at 2am (UTC)", now),
            Some(day(2026, 11, 16) + 2 * 3600)
        );
        assert_eq!(
            when("3:30pm (UTC)", now),
            Some(day(2026, 11, 10) + 15 * 3600 + 1800)
        );
        assert_eq!(
            when("4:59am (UTC)", now),
            Some(day(2026, 11, 11) + 4 * 3600 + 59 * 60)
        );
        assert_eq!(when("3pm.", now), Some(day(2026, 11, 10) + 15 * 3600));
        assert_eq!(
            when("Jan 2 at 12am (UTC)", day(2026, 12, 30)),
            Some(day(2027, 1, 2))
        );
        assert_eq!(when("in 2h 5m", now), Some(now + 2 * 3600 + 5 * 60));
        assert_eq!(when("Nov 16 at 2am (Asia/Bangkok)", now), None);
        assert_eq!(when("13:00pm", now), None);
        assert_eq!(when("in 2 weeks", now), None);
        assert_eq!(when("soon", now), None);
    }
}
