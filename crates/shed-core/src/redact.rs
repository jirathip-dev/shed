//! Tokens out of text a redacted terminal shows.

const SECRET_KEYS: [&str; 4] = ["token", "secret", "password", "key"];

fn tokenish(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-_.+=~".contains(c)
}

fn is_token(run: &str) -> bool {
    run.len() >= 20
        && run.contains(|c: char| c.is_ascii_digit())
        && run.contains(|c: char| c.is_ascii_alphabetic())
}

fn link(word: &str, out: &mut String) {
    for part in word.split_inclusive(['?', '&', '#']) {
        match part.split_once('=') {
            Some((k, v))
                if SECRET_KEYS
                    .iter()
                    .any(|s| k.to_ascii_lowercase().contains(s)) =>
            {
                let end = v.trim_end_matches(['&', '#']);
                out.extend([k, "=[redacted]", v.strip_prefix(end).unwrap_or_default()]);
            }
            _ => out.push_str(part),
        }
    }
}

fn words(word: &str, out: &mut String) {
    let mut run = String::new();
    for c in word.chars().map(Some).chain([None]) {
        if let Some(c) = c.filter(|&c| tokenish(c)) {
            run.push(c);
            continue;
        }
        out.push_str(if is_token(&run) { "[redacted]" } else { &run });
        run.clear();
        out.extend(c);
    }
}

/// `text` with anything that looks like a token replaced by `[redacted]`: a run of 20 or more token characters
/// with both a letter and a digit, and in a link the values of `token`, `secret`, `password` and `key` parameters.
#[must_use]
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split_inclusive(char::is_whitespace) {
        let bare = word.trim_end();
        if bare.starts_with("https://") || bare.starts_with("http://") {
            link(bare, &mut out);
        } else {
            words(bare, &mut out);
        }
        out.push_str(word.strip_prefix(bare).unwrap_or_default());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn redacts_tokens_keeps_words_paths_codes_and_links() {
        let token = format!("sk-{}", "a1".repeat(12));
        let jwt = format!("eyJ{}.{}", "x7".repeat(10), "y8".repeat(10));
        let screen = format!(
            "token: {token}\nid {jwt}.\nOpen https://example.com/auth?client_id=abc&state=s1&access_token={token}#x\ncode ABCD-1234 in ~/.config/shed\n"
        );
        assert_eq!(
            redact(&screen),
            "token: [redacted]\nid [redacted]\nOpen https://example.com/auth?client_id=abc&state=s1&access_token=[redacted]#x\ncode ABCD-1234 in ~/.config/shed\n"
        );
        let long_link = format!("https://example.com/a?code_challenge={}", "Q9".repeat(20));
        assert_eq!(redact(&long_link), long_link);
        assert_eq!(redact("Logged in using ChatGPT"), "Logged in using ChatGPT");
    }

    #[test]
    fn a_short_run_a_run_of_letters_and_a_run_of_digits_stay() {
        let digits = "1".repeat(30);
        let letters = "a".repeat(30);
        let text = format!("{digits} {letters} abc123 \"{}\"", "b2".repeat(10));
        assert_eq!(
            redact(&text),
            format!("{digits} {letters} abc123 \"[redacted]\"")
        );
    }

    #[test]
    fn a_google_sign_in_link_stays_whole_even_wrapped_across_screen_rows() {
        let link = "https://accounts.google.com/o/oauth2/v2/auth?redirect_uri=http%3A%2F%2F127.0.0.1%3A57865%2Fauth%2Faicode&access_type=offline&scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fcloud-platform%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fuserinfo.email&code_challenge=Xk3vQ9mB2rT7wLpZ8nY4cF6hJ1sD5gA0eU2iO3kP7qW&code_challenge_method=S256&state=9f2c4e7a1b3d5f6e8a0c2b4d6f8e1a3c&response_type=code&client_id=123456789012-a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6.apps.googleusercontent.com";
        let line = format!("Open the following link to authenticate the ACP server: {link}");
        assert_eq!(redact(&line), line);
        let rows: Vec<String> = line
            .chars()
            .collect::<Vec<_>>()
            .chunks(80)
            .map(String::from_iter)
            .collect();
        let screen = rows.join("\n");
        assert_eq!(redact(&screen), screen);
        let token = format!(
            "https://example.com/cb?state=s1&access_token={}",
            "a1".repeat(12)
        );
        assert_eq!(
            redact(&token),
            "https://example.com/cb?state=s1&access_token=[redacted]"
        );
    }
}
