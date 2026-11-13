//! Stamps the build with its git commit (`SHED_BUILD`), so client, server and `shed doctor` can tell builds apart.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    out.status.success().then(|| text.trim().to_owned())
}

fn main() {
    let mut watched = vec!["HEAD".to_owned(), "packed-refs".to_owned()];
    watched.extend(git(&["symbolic-ref", "-q", "HEAD"]));
    for path in watched {
        let file = git(&["rev-parse", "--path-format=absolute", "--git-path", &path]);
        if let Some(file) = file.filter(|f| std::path::Path::new(f).exists()) {
            println!("cargo:rerun-if-changed={file}");
        }
    }
    let sha = git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=SHED_BUILD={sha}");
}
