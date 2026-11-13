//! Key names (`enter`, `ctrl+c`, `up`, …) to the bytes a terminal sends for them.

use crate::Error;

const NAMED: &[(&str, &[u8])] = &[
    ("enter", b"\r"),
    ("tab", b"\t"),
    ("shift+tab", b"\x1b[Z"),
    ("esc", b"\x1b"),
    ("space", b" "),
    ("backspace", b"\x7f"),
    ("up", b"\x1b[A"),
    ("down", b"\x1b[B"),
    ("right", b"\x1b[C"),
    ("left", b"\x1b[D"),
    ("home", b"\x1b[H"),
    ("end", b"\x1b[F"),
    ("pageup", b"\x1b[5~"),
    ("pagedown", b"\x1b[6~"),
    ("delete", b"\x1b[3~"),
];

/// The bytes for one key: a name from the table above, `ctrl+<letter>` (or `ctrl-<letter>`), or one
/// literal character.
///
/// Names are case-insensitive; a single character is sent as itself (UTF-8).
///
/// # Errors
/// [`Error::UnknownKey`] for anything else.
pub fn encode(key: &str) -> Result<Vec<u8>, Error> {
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Ok(c.to_string().into_bytes());
    }
    let lower = key.to_ascii_lowercase();
    if let Some((_, bytes)) = NAMED.iter().find(|(name, _)| *name == lower) {
        return Ok(bytes.to_vec());
    }
    if let Some(&[c]) = (lower
        .strip_prefix("ctrl+")
        .or_else(|| lower.strip_prefix("ctrl-")))
    .map(str::as_bytes)
        && c.is_ascii_lowercase()
    {
        return Ok(vec![c & 0x1f]);
    }
    Err(Error::UnknownKey(key.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_encoding_table() {
        let table: &[(&str, &[u8])] = &[
            ("enter", b"\r"),
            ("Enter", b"\r"),
            ("esc", b"\x1b"),
            ("tab", b"\t"),
            ("shift+tab", b"\x1b[Z"),
            ("space", b" "),
            ("backspace", b"\x7f"),
            ("up", b"\x1b[A"),
            ("down", b"\x1b[B"),
            ("right", b"\x1b[C"),
            ("left", b"\x1b[D"),
            ("home", b"\x1b[H"),
            ("end", b"\x1b[F"),
            ("pageup", b"\x1b[5~"),
            ("pagedown", b"\x1b[6~"),
            ("delete", b"\x1b[3~"),
            ("ctrl+c", b"\x03"),
            ("ctrl+d", b"\x04"),
            ("CTRL+Z", b"\x1a"),
            ("Ctrl-x", b"\x18"),
            ("a", b"a"),
            ("é", "é".as_bytes()),
        ];
        for (key, bytes) in table {
            assert_eq!(encode(key).ok().as_deref(), Some(*bytes), "{key}");
        }
    }

    #[test]
    fn unknown_keys_are_errors() {
        for key in ["", "ctrl+", "ctrl+1", "ctrl+ab", "hyper"] {
            assert!(
                matches!(encode(key), Err(Error::UnknownKey(k)) if k == key),
                "{key}"
            );
        }
    }
}
