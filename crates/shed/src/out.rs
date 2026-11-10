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
