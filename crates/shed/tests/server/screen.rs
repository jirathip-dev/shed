use std::io::Write as _;

use serde_json::{Value, json};

use crate::common::{Server, eventually, temp_dir};
use crate::{Conn, hello, rows, shell};

fn until_row(watch: &mut Conn, text: &str) -> Vec<Value> {
    let mut updates = Vec::new();
    loop {
        let update = watch.event();
        let done = rows(&update).iter().any(|(_, t)| t == text);
        updates.push(update);
        if done {
            return updates;
        }
    }
}

#[test]
fn screen_watch_sends_the_full_screen_then_diffs_and_since_skips_the_full_one() {
    let dir = temp_dir("watch");
    let _server = Server::start(&dir);
    let mut c = hello(&dir);
    shell(&mut c, &dir, "w");

    let mut watch = hello(&dir);
    assert_eq!(
        watch.result("screen.watch", &json!({"terminal": "w"})),
        json!({})
    );
    let first = watch.event();
    assert_eq!(first["full"], true);
    assert_eq!((&first["rows"], &first["cols"]), (&json!(50), &json!(200)));
    assert_eq!(first["cursor"], json!([0, 2]));
    let all = rows(&first);
    assert_eq!(all.len(), 50);
    assert_eq!(all[0], (0, "$".to_owned()));

    c.result(
        "terminal.input",
        &json!({"terminal": "w", "text": "printf '\\033[1;31mred\\033[0m\\n'\n"}),
    );
    let updates = until_row(&mut watch, "red");
    let last = updates.last().expect("an update");
    assert!(updates.iter().all(|u| u["full"] == false), "{updates:?}");
    assert!(rows(last).len() < 50, "only changed rows: {last}");
    let red = &last["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .find(|l| l["row"] == 1)
        .expect("row 1")["runs"][0];
    assert_eq!(
        red,
        &json!({"text": "red", "fg": 1, "bg": null, "bold": true, "italic": false, "underline": false, "inverse": false})
    );

    eventually("the prompt after the output", || {
        c.result("read", &json!({"name": "w"}))["text"]
            .as_str()
            .is_some_and(|t| t.ends_with("red\n$"))
    });
    let mut again = hello(&dir);
    again.result("screen.watch", &json!({"terminal": "w"}));
    let seq = again.event()["seq"].clone();
    let mut since = hello(&dir);
    since.result("screen.watch", &json!({"terminal": "w", "since": seq}));
    c.result("terminal.input", &json!({"terminal": "w", "text": "x"}));
    let next = since.event();
    assert_eq!(next["full"], false, "{next}");
    assert_eq!(rows(&next), [(2, "$ x".to_owned())]);
}

#[test]
fn screen_watch_sends_at_most_20_updates_a_second() {
    let dir = temp_dir("throttle");
    let _server = Server::start(&dir);
    let mut c = hello(&dir);
    shell(&mut c, &dir, "w");
    let mut watch = hello(&dir);
    watch.result("screen.watch", &json!({"terminal": "w"}));
    watch.event();

    let start = std::time::Instant::now();
    let flood = "i=0; while [ $i -lt 3000 ]; do echo l$i; i=$((i+1)); done; echo end\n";
    c.result("terminal.input", &json!({"terminal": "w", "text": flood}));
    let n = until_row(&mut watch, "end").len();
    let elapsed = start.elapsed().as_millis();
    assert!(
        (u128::try_from(n).unwrap() - 1) * 50 <= elapsed,
        "{n} updates in {elapsed} ms"
    );
}

#[test]
fn terminal_resize_from_the_watcher_resizes_the_screen_and_the_program() {
    let dir = temp_dir("resize");
    let _server = Server::start(&dir);
    let mut c = hello(&dir);
    shell(&mut c, &dir, "w");
    let mut watch = hello(&dir);
    watch.result("screen.watch", &json!({"terminal": "w"}));
    assert_eq!(watch.event()["cols"], 200);

    let resize = json!({"jsonrpc": "2.0", "id": 9, "method": "terminal.resize", "params": {"cols": 40, "rows": 10}});
    watch
        .writer
        .write_all(format!("{resize}\n").as_bytes())
        .expect("write");
    let resized = watch.event();
    assert_eq!(
        (&resized["full"], &resized["rows"], &resized["cols"]),
        (&json!(true), &json!(10), &json!(40)),
        "{resized}"
    );
    assert_eq!(rows(&resized).len(), 10);

    c.result(
        "terminal.input",
        &json!({"terminal": "w", "text": "stty size\n"}),
    );
    until_row(&mut watch, "10 40");
    let (_, _, none) = c.call(1, "terminal.resize", &json!({"cols": 40, "rows": 10}));
    assert_eq!(none["error"]["code"], -32602);
}
