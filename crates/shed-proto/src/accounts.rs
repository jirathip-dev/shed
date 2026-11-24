//! The accounts' keys of a provider record, and what its status command and usage probe say.

use std::collections::BTreeMap;
use std::path::PathBuf;

#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Provider, Tokens};

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

/// The balance probe: the API that answers an account's balance for its key, and where the answer holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct BalanceProbe {
    /// The URL.
    pub url: String,
    /// The header that carries the key, `{token}` standing for it.
    pub auth: String,
    /// The keys and indexes down to the currency.
    pub currency: Vec<Value>,
    /// The keys and indexes down to the total, a number or a numeric string.
    pub total: Vec<Value>,
    /// Seconds a reading stands.
    pub fresh: u64,
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

/// Where a login lives besides its status command: a file in the config folder and a keychain item, whose
/// sha256 are compared for drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct CredentialsRecord {
    /// The file under the config folder.
    pub file: String,
    /// The default config folder, under the home folder.
    pub default_dir: String,
    /// The keychain item's service for the default folder.
    pub service: String,
    /// A named folder's service: `service`, `-`, then this many hex digits of the folder path's sha256.
    pub folder_hash: usize,
    /// The command that prints an item's secret, `{service}` standing for its service.
    pub keychain: Vec<String>,
    /// The command's exit status when the item is missing.
    pub missing: i32,
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

/// A balance reading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Balance {
    /// Its currency.
    pub currency: String,
    /// The total left.
    pub total: f64,
    /// When it was read.
    pub read_at: u64,
}

/// What a million tokens of each kind cost, for agents whose usage carries no cost.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Price {
    /// Its currency.
    pub currency: String,
    /// Input tokens neither read from nor written to the prompt cache.
    pub input: f64,
    /// Input tokens read from the prompt cache.
    pub cache_read: f64,
    /// Input tokens written to the prompt cache.
    pub cache_write: f64,
    /// Output tokens.
    pub output: f64,
}

impl Eq for Price {}

impl Price {
    /// What `tokens` cost.
    #[must_use]
    pub fn of(&self, tokens: Tokens) -> Money {
        let m = |n: u64, per: f64| f64::from(u32::try_from(n).unwrap_or(u32::MAX)) * per / 1e6;
        let amount = m(tokens.input, self.input)
            + m(tokens.cache_read, self.cache_read)
            + m(tokens.cache_write, self.cache_write)
            + m(tokens.output, self.output);
        Money {
            amount,
            currency: self.currency.clone(),
        }
    }
}

/// An amount in a currency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Money {
    /// The amount.
    pub amount: f64,
    /// Its currency.
    pub currency: String,
}

/// A cost: an amount, or `"unknown"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(untagged)]
pub enum Cost {
    /// What it cost.
    Known(Money),
    /// Some usage had neither its own cost nor a price, or the currencies differ.
    Unknown(Unknown),
}

/// The `"unknown"` cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Unknown {
    /// `"unknown"`.
    Unknown,
}

impl Cost {
    fn of(cost: Option<Money>) -> Self {
        cost.map_or(Self::Unknown(Unknown::Unknown), Self::Known)
    }
}

/// An API-key account's tokens and cost in one calendar month on one machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Spend {
    /// `YYYY-MM`, in the machine's local time.
    pub month: String,
    /// The tokens its lanes used.
    pub tokens: Tokens,
    /// What they cost.
    pub cost: Cost,
}

impl Spend {
    /// `self` with `tokens` costing `cost` added in `month`; another month starts again from them.
    #[must_use]
    pub fn plus(self, month: &str, tokens: Tokens, cost: Option<Money>) -> Self {
        if self.month != month {
            return Self::new(month, tokens, cost);
        }
        let cost = match (self.cost, cost) {
            (Cost::Known(a), Some(b)) if a.currency == b.currency => Cost::Known(Money {
                amount: a.amount + b.amount,
                currency: a.currency,
            }),
            _ => Cost::of(None),
        };
        Self {
            tokens: self.tokens.plus(tokens),
            cost,
            ..self
        }
    }

    /// `tokens` costing `cost` in `month`.
    #[must_use]
    pub fn new(month: &str, tokens: Tokens, cost: Option<Money>) -> Self {
        Self {
            month: month.to_owned(),
            tokens,
            cost: Cost::of(cost),
        }
    }
}

/// An account: a provider's login in a config folder shed keeps, and what shed last read of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Signed out, ready, limited or drifted.
    pub state: AccountState,
    /// The email or organisation the status command reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub who: Option<String>,
    /// The status on one line, redacted.
    #[serde(default)]
    pub line: String,
    /// Whether its keychain item and credentials file agree; none without a check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drift: Option<Drift>,
    /// When the status command last ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<u64>,
    /// While `state` is `limited`: when the limit ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limited_until: Option<u64>,
    /// The last usage reading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// The last balance reading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<Balance>,
    /// An API-key account's tokens and cost this month on this machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend: Option<Spend>,
    /// Its setup token's dates, never the token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<TokenInfo>,
    /// The record's `setup_files` found in its folder; not empty: no run.
    #[serde(default)]
    pub setup: Vec<String>,
    /// The terminals on it now.
    #[serde(default)]
    pub runs: u32,
    /// Its login terminal, while one runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    /// `auto` picks it first while it is ready (`account.use`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preferred: bool,
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
    /// Its keychain item and credentials file hold different logins.
    Drifted,
}

/// What the drift check found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum Drift {
    /// They agree, or one of them is missing.
    Clear,
    /// Both are there and differ.
    Drifted,
    /// The keychain could not be read.
    Unknown,
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

/// `account.use` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct AccountUse {
    /// The record's `agent` name.
    pub provider: String,
    /// The named account to prefer and move the lanes to; none drops the preference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// `account.use`'s result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct AccountMoves {
    /// The lanes told to move.
    pub moved: Vec<String>,
    /// The provider's terminals that stay, and why.
    pub stayed: Vec<Stayed>,
}

/// A terminal `account.use` leaves where it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Stayed {
    /// The terminal.
    pub terminal: String,
    /// Why it stays.
    pub why: String,
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

/// `account.login` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct AccountLogin {
    /// The record's `agent` name.
    pub provider: String,
    /// The account.
    pub name: String,
    /// Run the record's `token.command` and save the setup token it prints (0600).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub token: bool,
}

/// A per-run key of the token proxy; never the key itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ProxyKey {
    /// The first 8 hex digits of its sha256.
    pub id: String,
    /// `<machine>/<name>`: the run it was made for.
    pub terminal: String,
    /// The record's `agent` name.
    pub provider: String,
    /// The account whose token signs its calls now.
    pub account: String,
    /// When it was made.
    pub created: u64,
    /// Live, revoked or ended.
    pub state: KeyState,
}

/// A per-run key's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum KeyState {
    /// It signs its run's calls.
    Live,
    /// `proxy.revoke` ended it.
    Revoked,
    /// Its terminal exited, was killed or was removed.
    Ended,
}

/// `proxy.revoke` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct KeyId {
    /// The key's id.
    pub id: String,
}

/// `proxy.relay`'s result: where a machine's agents reach the token proxy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Relay {
    /// The loopback port the machine's relay keeps.
    pub port: u16,
    /// The socket path the next forward binds there.
    pub socket: PathBuf,
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

    /// The balance probe's reading of the API's answer `body` at `now`; none without a probe or when it does
    /// not read.
    #[must_use]
    pub fn read_balance(&self, body: &str, now: u64) -> Option<Balance> {
        let probe = self.balance.as_ref()?;
        let answer: Value = serde_json::from_str(body).ok()?;
        let at = |path: &[Value]| {
            path.iter().try_fold(&answer, |v, step| match step {
                Value::String(key) => v.get(key),
                Value::Number(i) => v.get(usize::try_from(i.as_u64()?).ok()?),
                _ => None,
            })
        };
        let total = match at(&probe.total)? {
            Value::String(text) => text.trim().parse().ok()?,
            number => number.as_f64()?,
        };
        Some(Balance {
            currency: at(&probe.currency)?.as_str()?.to_owned(),
            total,
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
