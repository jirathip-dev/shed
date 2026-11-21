//! The prompt framing: what `send` types into an ACP bridge's terminal.

use serde::{Deserialize, Serialize};

/// One framed input to the bridge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Input {
    /// Start a turn (`session/prompt`).
    Prompt(String),
    /// Cancel the turn (`session/cancel`).
    Cancel,
    /// Switch to this model (`session/set_config_option`, the `model` option).
    Model(String),
    /// Report under this terminal name from now on: a spare session taken by `run --warm`.
    Name(String),
    /// Answer permission request `request` (else the oldest) with this option id or kind.
    #[serde(untagged)]
    Approve {
        /// The option id or kind.
        #[serde(rename = "approve")]
        option: String,
        /// The `approval_request`'s id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request: Option<String>,
    },
}

/// The bytes that deliver `input` to a bridge through its terminal: `RS`, the JSON, a newline.
#[must_use]
pub fn frame(input: &Input) -> String {
    format!("\x1e{}\n", serde_json::to_string(input).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_one_line_of_json() {
        let framed = frame(&Input::Prompt("a\nb".to_owned()));
        assert_eq!(framed, "\x1e{\"prompt\":\"a\\nb\"}\n");
        assert_eq!(frame(&Input::Cancel), "\x1e\"cancel\"\n");
        let approve = |request: Option<&str>| Input::Approve {
            option: "allow".to_owned(),
            request: request.map(str::to_owned),
        };
        let line = "{\"approve\":\"allow\",\"request\":\"7\"}";
        assert_eq!(frame(&approve(Some("7"))), format!("\x1e{line}\n"));
        let read = |l: &str| serde_json::from_str::<Input>(l).unwrap();
        assert_eq!(read(line), approve(Some("7")));
        assert_eq!(read("{\"approve\":\"allow\"}"), approve(None));
    }
}
