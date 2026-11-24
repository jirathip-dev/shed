use crate::common::{Server, temp_dir};
use crate::{shed_in, stdout};

#[test]
fn wait_matches_output_by_pattern_and_falls_back_to_idle() {
    let dir = temp_dir("wait-output");
    let d = dir.to_str().expect("utf-8 dir");
    let _server = Server::start(&dir);
    let script = "sleep 0.3; echo 'ok 12 tests'; exec sleep 100";
    stdout(&shed_in(
        &dir,
        &[
            "run", "--name", "test", "--cwd", d, "--", "sh", "-c", script,
        ],
    ));
    let out = shed_in(
        &dir,
        &[
            "wait",
            "test",
            "--output",
            "^ok \\d+ tests$",
            "--timeout",
            "10",
        ],
    );
    assert_eq!(stdout(&out), "test output\n");

    let idle = [
        "run",
        "--name",
        "quiet",
        "--idle-after",
        "1",
        "--cwd",
        d,
        "--",
        "sleep",
        "100",
    ];
    stdout(&shed_in(&dir, &idle));
    let out = shed_in(
        &dir,
        &["wait", "quiet", "--until", "idle", "--timeout", "10"],
    );
    assert_eq!(stdout(&out), "quiet idle\n");
    let out = shed_in(
        &dir,
        &["wait", "test", "--until", "idle", "--timeout", "1.2"],
    );
    assert_eq!(
        out.status.code(),
        Some(124),
        "no fallback unless asked: {out:?}"
    );
}
