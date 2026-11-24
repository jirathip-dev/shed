//! Transcripts as text: [`render`] for `read`, [`Preview`] for a reader's terminal.

use std::collections::BTreeMap;
use std::io;

use shed_proto::EventKind;

pub(super) fn clean(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' | '\t' => c,
            c if c.is_control() => '?',
            c => c,
        })
        .collect()
}

/// A transcript (JSONL events) as text: each prompt (`> …`), one line per tool call with its last
/// status, and each turn's final reply.
#[must_use]
pub fn render(transcript: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut tools: BTreeMap<String, usize> = BTreeMap::new();
    for kind in transcript
        .lines()
        .filter_map(|l| serde_json::from_str::<EventKind>(l).ok())
    {
        match kind {
            EventKind::Turn { prompt, .. } => lines.push(format!("> {prompt}")),
            EventKind::ToolCall {
                id, title, status, ..
            } => {
                let line = format!("[tool] {title} ({status})");
                if let Some(slot) = tools.get(&id).and_then(|&at| lines.get_mut(at)) {
                    *slot = line;
                } else {
                    tools.insert(id, lines.len());
                    lines.push(line);
                }
            }
            EventKind::Reply { text, .. } if !text.is_empty() => lines.push(text),
            _ => {}
        }
    }
    clean(&lines.join("\n"))
}

/// A transcript drawn for a reader: each block styled and wrapped, every tool call on one line at its
/// first place with its last status. Only ACP's own fields are read; an unknown status gets no glyph.
#[derive(Default)]
pub struct Preview {
    pub(super) blocks: Vec<Result<EventKind, String>>,
    tools: BTreeMap<String, usize>,
}

impl Preview {
    /// Adds an event; `false` when it draws nothing.
    pub fn push(&mut self, kind: EventKind) -> bool {
        match &kind {
            EventKind::ToolCall { id, .. } if self.tools.contains_key(id) => {
                let at = self.tools.get(id).copied();
                if let Some(block) = at.and_then(|at| self.blocks.get_mut(at)) {
                    *block = Ok(kind);
                }
                return true;
            }
            EventKind::ToolCall { id, .. } => _ = self.tools.insert(id.clone(), self.blocks.len()),
            EventKind::Reply { text, .. } if text.is_empty() => return false,
            EventKind::Turn { .. }
            | EventKind::Reply { .. }
            | EventKind::Plan { .. }
            | EventKind::ApprovalRequest { .. }
            | EventKind::Approved { .. } => {}
            _ => return false,
        }
        self.blocks.push(Ok(kind));
        true
    }

    /// The last `rows` lines, `width` columns wide, with SGR colours when `color`.
    #[must_use]
    pub fn draw(&self, width: usize, color: bool, rows: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for block in self.blocks.iter().rev() {
            if lines.len() >= rows {
                break;
            }
            let mut out = Vec::new();
            let mut put = |sgr: &str, first: &str, rest: &str, text: &str| {
                let room = width.saturating_sub(first.chars().count()).max(1);
                let text = clean(text);
                for (i, l) in text.split('\n').flat_map(|l| wrap(l, room)).enumerate() {
                    let lead = if i == 0 { first } else { rest };
                    out.push(match (color, sgr) {
                        (false, _) => format!("{lead}{l}"),
                        (true, "") => format!("{lead}{}", code(&l)),
                        (true, _) => format!("{lead}\x1b[{sgr}m{l}\x1b[0m"),
                    });
                }
            };
            match block {
                Err(line) => put("", "", "", line),
                Ok(EventKind::Turn { prompt, .. }) => put("1", "› ", "  ", prompt),
                Ok(EventKind::Reply { text, .. }) => markdown(text, &mut put),
                Ok(EventKind::ToolCall {
                    title,
                    status,
                    kind,
                    ..
                }) => {
                    let glyph = mark(status).0;
                    let kind = kind.as_deref().map(|k| format!("{k} ")).unwrap_or_default();
                    let line = format!("{glyph}{kind}{}", title.lines().next().unwrap_or(""));
                    put("2", "", "", &line.chars().take(width).collect::<String>());
                }
                Ok(EventKind::Plan { entries }) => {
                    for e in entries {
                        let mark = mark(e["status"].as_str().unwrap_or("")).1;
                        put("", mark, "    ", e["content"].as_str().unwrap_or(""));
                    }
                }
                Ok(EventKind::ApprovalRequest { tool, options, .. }) => {
                    let options = options.iter().map(|o| format!("{} ({})", o.id, o.name));
                    let options = options.collect::<Vec<_>>().join(" | ");
                    let tool = tool.lines().next().unwrap_or("");
                    put("33", "? ", "  ", &format!("{tool}: {options}"));
                }
                Ok(EventKind::Approved {
                    option,
                    decision,
                    reason,
                    ..
                }) => {
                    let judged = decision.as_ref().map(|d| {
                        format!(" (judge: {d}, {})", reason.as_deref().unwrap_or_default())
                    });
                    put(
                        "2",
                        "",
                        "",
                        &format!("approved {option}{}", judged.unwrap_or_default()),
                    );
                }
                Ok(_) => {}
            }
            lines.extend(out.into_iter().rev());
        }
        lines.truncate(rows);
        lines.reverse();
        lines
    }
}

fn mark(status: &str) -> (&'static str, &'static str) {
    [
        ("pending", "… ", "[ ] "),
        ("in_progress", "… ", "[~] "),
        ("completed", "✓ ", "[x] "),
        ("failed", "✗ ", "[!] "),
    ]
    .into_iter()
    .find(|(s, ..)| *s == status)
    .map_or(("", "[ ] "), |(_, glyph, text)| (glyph, text))
}

fn markdown(text: &str, put: &mut impl FnMut(&str, &str, &str, &str)) {
    let mut fence = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        let numbered = trimmed.split_once(". ").map(|(n, _)| n);
        let item = trimmed
            .strip_prefix(['-', '*', '+'])
            .filter(|r| r.starts_with(' '));
        if let Some(lang) = trimmed.strip_prefix("```") {
            fence = !fence;
            if fence && !lang.is_empty() {
                put("2", "    ", "    ", lang);
            }
        } else if fence {
            put("2", "    ", "    ", line);
        } else if let Some(h) = trimmed.strip_prefix('#') {
            put("1", "", "", h.trim_start_matches('#').trim_start());
        } else if let Some(item) = item {
            put("", "  •", "    ", item);
        } else if numbered.is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
            put("", "  ", "     ", trimmed);
        } else {
            put("", "", "", line);
        }
    }
}

fn code(line: &str) -> String {
    let parts = line.split('`').enumerate();
    let parts = parts.map(|(i, p)| match i % 2 {
        1 => format!("\x1b[36m{p}\x1b[39m"),
        _ => p.to_owned(),
    });
    parts.collect()
}

fn wrap(line: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    for (i, word) in line.split(' ').enumerate() {
        if i > 0
            && let Some(last) = lines.last_mut()
            && last.chars().count() + 1 + word.chars().count() <= width
        {
            last.push(' ');
        } else if i > 0 {
            lines.push(String::new());
        }
        for (j, chunk) in word.chars().collect::<Vec<_>>().chunks(width).enumerate() {
            if j > 0 {
                lines.push(String::new());
            }
            if let Some(last) = lines.last_mut() {
                last.extend(chunk);
            }
        }
    }
    lines
}

/// Columns, rows and colour for drawing to stdout: 80 × 24 unless it is a terminal, and colour only
/// on a terminal without `NO_COLOR`.
#[must_use]
pub fn stdout_style() -> (usize, usize, bool) {
    let tty = rustix::termios::isatty(io::stdout());
    let size = (rustix::termios::tcgetwinsize(io::stdout()).ok()).filter(|_| tty);
    let size = size.map_or((80, 24), |s| (s.ws_col.max(20), s.ws_row.max(2)));
    (
        size.0.into(),
        size.1.into(),
        tty && crate::config::var_os("NO_COLOR").is_none(),
    )
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn the_preview_draws_an_unknown_status_plain_and_keeps_each_call_on_its_first_line() {
        let call = |id: &str, status: &str, kind: Option<&str>| EventKind::ToolCall {
            id: id.into(),
            title: "line one\nline two".into(),
            status: status.into(),
            kind: kind.map(Into::into),
            locations: Vec::new(),
            raw_input: None,
        };
        let mut preview = Preview::default();
        assert!(preview.push(call("a", "pending", Some("execute"))));
        assert!(preview.push(call("b", "teleported", Some("warp"))));
        assert!(!preview.push(EventKind::Text { text: "x".into() }));
        assert!(preview.push(call("a", "failed", None)));
        assert_eq!(preview.draw(12, false, 9), ["✗ line one", "warp line on"]);
        assert_eq!(preview.draw(40, false, 1), ["warp line one"]);
        let styled = preview.draw(40, true, 9);
        assert_eq!(styled[0], "\x1b[2m✗ line one\x1b[0m");
    }

    #[test]
    fn render_keeps_prompts_one_line_per_tool_call_and_replies() {
        let lines = [
            json!({"event": "turn", "prompt": "fix it"}),
            json!({"event": "text", "text": "Look"}),
            json!({"event": "tool_call", "id": "a", "title": "Edit", "status": "pending"}),
            json!({"event": "tool_call", "id": "b", "title": "Run", "status": "pending"}),
            json!({"event": "tool_call", "id": "a", "title": "Edit", "status": "completed"}),
            json!({"event": "reply", "text": "Done\u{1b}[2J", "stop_reason": "end_turn"}),
        ];
        let jsonl: Vec<String> = lines.iter().map(Value::to_string).collect();
        assert_eq!(
            render(&jsonl.join("\n")),
            "> fix it\n[tool] Edit (completed)\n[tool] Run (pending)\nDone?[2J"
        );
    }
}
