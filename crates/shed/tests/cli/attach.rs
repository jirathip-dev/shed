use std::time::Duration;

use shed_core::{Size, Terminal};

use crate::common::{Server, eventually, temp_dir};
use crate::{SHED, shed_in, stdout};

#[test]
fn attach_types_resizes_and_detaches_in_a_pty() {
    let dir = temp_dir("attach");
    let d = dir.to_str().expect("utf-8 dir");
    let _server = Server::start(&dir);
    stdout(&shed_in(
        &dir,
        &["run", "--name", "shell", "--cwd", d, "--", "sh"],
    ));
    eventually("the prompt", || {
        stdout(&shed_in(&dir, &["read", "shell"])) == "$\n"
    });

    let size = Size { rows: 24, cols: 80 };
    let cmd = [SHED, "--dir", d, "attach", "shell"];
    let outer = Terminal::spawn(&cmd, &dir, [("TERM", "xterm")], size).expect("attach in a pty");
    eventually("the redraw", || outer.screen().starts_with('$'));
    outer.write(b"echo hi-$((1+2))\r").expect("type");
    eventually("typed input to run", || outer.screen().contains("hi-3"));

    outer
        .resize(Size {
            rows: 30,
            cols: 100,
        })
        .expect("resize");
    let forwarded = shed_server::clock::poll(shed_server::clock::PATIENCE, || {
        outer.write(b"stty size\r").expect("type");
        shed_server::clock::sleep(Duration::from_millis(300));
        outer.screen().contains("30 100")
    });
    assert!(forwarded, "resize not forwarded: {}", outer.screen());

    outer.write(&[0x1c]).expect("Ctrl+\\");
    eventually("detach", || matches!(outer.try_wait(), Ok(Some(_))));
    let status = outer.try_wait().expect("status").expect("exited");
    assert!(status.success(), "attach exits 0 on detach: {status:?}");
    let ls = stdout(&shed_in(&dir, &["ls"]));
    assert!(
        ls.contains("shell  running"),
        "still running after detach: {ls}"
    );
    let read = stdout(&shed_in(&dir, &["read", "shell", "--history"]));
    assert!(read.contains("hi-3"), "{read}");
}

#[test]
fn a_query_the_screen_answers_reaches_the_program_once_with_a_client_attached() {
    let dir = temp_dir("attach-da1");
    let d = dir.to_str().expect("utf-8 dir");
    let _server = Server::start(&dir);
    let script = r#"stty -echo; printf ready; read go; printf '\033[casked'; read line
printf '\nreplies=%d\n' "$(printf %s "$line" | tr -cd c | wc -c)"; exec sleep 100"#;
    stdout(&shed_in(
        &dir,
        &["run", "--name", "q", "--cwd", d, "--", "sh", "-c", script],
    ));
    let size = Size { rows: 24, cols: 80 };
    let cmd = [SHED, "--dir", d, "attach", "q"];
    let outer = Terminal::spawn(&cmd, &dir, [("TERM", "xterm")], size).expect("attach in a pty");
    eventually("the program", || outer.screen().contains("ready"));
    outer.write(b"go\r").expect("type");
    eventually("the query", || outer.screen().contains("asked"));
    outer.write(b"\r").expect("type");
    eventually("the count", || {
        stdout(&shed_in(&dir, &["read", "q"])).contains("replies=")
    });
    let read = stdout(&shed_in(&dir, &["read", "q"]));
    assert!(read.contains("replies=1\n"), "{read}");
}

#[test]
fn run_size_sets_the_program_size_and_the_default_is_wide() {
    let dir = temp_dir("run-size");
    let d = dir.to_str().expect("utf-8 dir");
    let _server = Server::start(&dir);
    let script = "stty size; exec sleep 100";
    for (name, size, want) in [
        ("wide", None, "50 200"),
        ("given", Some("50x200"), "50 200"),
        ("small", Some("7x33"), "7 33"),
    ] {
        let mut args = vec!["run", "--name", name, "--cwd", d];
        args.extend(size.map(|s| ["--size", s]).into_iter().flatten());
        args.extend(["--", "sh", "-c", script]);
        stdout(&shed_in(&dir, &args));
        eventually(want, || {
            stdout(&shed_in(&dir, &["read", name])) == format!("{want}\n")
        });
    }
    let bad = shed_in(&dir, &["run", "--size", "0x10", "--", "true"]);
    assert!(!bad.status.success(), "{bad:?}");
}

#[test]
fn attach_ends_when_the_program_exits() {
    let dir = temp_dir("attach-exit");
    let d = dir.to_str().expect("utf-8 dir");
    let _server = Server::start(&dir);
    stdout(&shed_in(
        &dir,
        &["run", "--name", "cat", "--cwd", d, "--", "cat"],
    ));
    let size = Size { rows: 24, cols: 80 };
    let cmd = [SHED, "--dir", d, "attach", "cat"];
    let outer = Terminal::spawn(&cmd, &dir, [("TERM", "xterm")], size).expect("attach in a pty");
    outer.write(b"ping\r").expect("type");
    eventually("the echo", || outer.screen().contains("ping"));
    stdout(&shed_in(&dir, &["kill", "cat"]));
    eventually("attach to end", || matches!(outer.try_wait(), Ok(Some(_))));
}
