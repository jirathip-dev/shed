use serde_json::json;

use crate::common::{Server, eventually, temp_dir};
use crate::{hello, shell};

#[test]
fn terminal_input_and_keys_arrive_and_a_readonly_client_is_refused() {
    let dir = temp_dir("input");
    let _server = Server::start(&dir);
    let mut c = hello(&dir);
    shell(&mut c, &dir, "w");
    c.result(
        "terminal.input",
        &json!({"terminal": "w", "text": "echo typed"}),
    );
    c.result(
        "terminal.keys",
        &json!({"terminal": "w", "keys": ["Esc", "Tab", "Left", "Right", "Enter"]}),
    );
    eventually("the echo", || {
        c.result("read", &json!({"name": "w"}))["text"]
            .as_str()
            .is_some_and(|t| t.contains("\ntyped\n"))
    });
    let (_, _, bad) = c.call(
        1,
        "terminal.keys",
        &json!({"terminal": "w", "keys": ["hyper"]}),
    );
    assert_eq!(bad["error"]["code"], -32602);

    let mut viewer = hello(&dir);
    assert_eq!(
        viewer.result("terminal.readonly", &json!({"terminal": "w"})),
        json!({})
    );
    let refused = [
        (
            "terminal.input",
            json!({"terminal": "w", "text": "echo nope\n"}),
        ),
        (
            "terminal.keys",
            json!({"terminal": "w", "keys": ["ctrl-c"]}),
        ),
        (
            "terminal.resize",
            json!({"terminal": "w", "cols": 10, "rows": 5}),
        ),
        ("send", json!({"name": "w", "text": "echo nope\n"})),
        ("keys", json!({"name": "w", "keys": ["enter"]})),
        ("resize", json!({"name": "w", "cols": 10, "rows": 5})),
        (
            "resize",
            json!({"name": "w", "terminal": "other", "cols": 10, "rows": 5}),
        ),
        ("attach", json!({"name": "w"})),
        (
            "terminal.resize",
            json!({"name": "w", "cols": 10, "rows": 5}),
        ),
    ];
    for (method, params) in refused {
        let (_, line, _) = viewer.call(1, method, &params);
        assert!(
            line.contains(r#""code":-32006,"message":"READONLY: w""#),
            "{method}: {line}"
        );
    }
    let (_, _, missing) = viewer.call(1, "terminal.readonly", &json!({"terminal": "nope"}));
    assert_eq!(missing["error"]["code"], -32002);

    c.result(
        "terminal.input",
        &json!({"terminal": "w", "text": "echo done\n"}),
    );
    eventually("the owner's echo", || {
        c.result("read", &json!({"name": "w"}))["text"]
            .as_str()
            .is_some_and(|t| t.contains("\ndone\n"))
    });
    let text = c.result("read", &json!({"name": "w"}))["text"].clone();
    assert!(!text.as_str().is_some_and(|t| t.contains("nope")), "{text}");
    let size = c.result("get", &json!({"name": "w"}));
    assert_eq!(size["state"], "running");
}

#[test]
fn fifty_concurrent_multi_line_sends_each_arrive_whole() {
    let dir = temp_dir("sends");
    let _server = Server::start(&dir);
    let mut c = hello(&dir);
    let cmd = ["sh", "-c", "exec cat > got.txt"];
    c.result("run", &json!({"name": "e", "cwd": dir, "cmd": cmd}));
    let mut texts: Vec<String> = (0..50)
        .map(|i| format!("{i} one\n{i} two\n{i} three"))
        .collect();
    std::thread::scope(|s| {
        for text in &texts {
            let dir = &dir;
            s.spawn(move || hello(dir).result("send", &json!({"name": "e", "text": text})));
        }
    });
    let got = || std::fs::read_to_string(dir.join("got.txt")).unwrap_or_default();
    eventually("every line", || got().lines().count() == 150);
    let got = got();
    let lines: Vec<&str> = got.lines().collect();
    let mut arrived: Vec<String> = lines.chunks(3).map(|t| t.join("\n")).collect();
    arrived.sort();
    texts.sort();
    assert_eq!(arrived, texts, "{got}");
}

#[test]
fn a_send_is_one_bracketed_paste_and_one_enter_only_when_the_program_turned_paste_on() {
    let dir = temp_dir("paste");
    let _server = Server::start(&dir);
    let mut c = hello(&dir);
    for (name, mode) in [("on", "\\033[?2004h"), ("off", "")] {
        let script = format!("stty raw -echo; printf '{mode}ready'; exec cat > {name}.bin");
        let cmd = ["sh", "-c", &script];
        c.result("run", &json!({"name": name, "cwd": dir, "cmd": cmd}));
        eventually("ready", || {
            c.result("read", &json!({"name": name}))["text"] == "ready"
        });
        c.result("send", &json!({"name": name, "text": "one\ntwo\nthree"}));
    }
    let bytes = |name: &str| std::fs::read(dir.join(format!("{name}.bin"))).unwrap_or_default();
    let pasted = b"\x1b[200~one\ntwo\nthree\x1b[201~\r";
    eventually("the paste", || bytes("on") == pasted);
    eventually("the typed text", || bytes("off") == b"one\ntwo\nthree\r");
    let options = json!({"name": "on", "text": "four", "no_paste": true, "no_enter": true});
    c.result("send", &options);
    eventually("the plain text", || {
        bytes("on") == [&pasted[..], b"four"].concat()
    });
}
