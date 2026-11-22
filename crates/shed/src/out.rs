use std::fmt::Display;

#[expect(clippy::print_stdout, reason = "the CLI's one stdout writer")]
pub fn text(s: impl Display) {
    print!("{s}");
}

pub fn line(s: impl Display) {
    text(format_args!("{s}\n"));
}

#[expect(clippy::print_stderr, reason = "the CLI's one stderr writer")]
pub fn err(s: impl Display) {
    eprintln!("{s}");
}

pub fn print<T: serde::Serialize>(
    json: bool,
    value: &T,
    text: impl FnOnce() -> String,
) -> anyhow::Result<()> {
    if json {
        line(serde_json::to_string(value)?);
    } else {
        self::text(text());
    }
    Ok(())
}

pub fn raw<T: serde::Serialize>(
    json: bool,
    value: &T,
    stdout: &[u8],
    stderr: &[u8],
) -> anyhow::Result<()> {
    use std::io::Write as _;
    if json {
        line(serde_json::to_string(value)?);
        return Ok(());
    }
    std::io::stdout().write_all(stdout)?;
    std::io::stdout().flush()?;
    std::io::stderr().write_all(stderr)?;
    Ok(())
}
