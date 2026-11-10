pub fn list() {
    let _ = std::process::Command::new("ls").status();
}
