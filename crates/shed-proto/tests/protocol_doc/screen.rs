use serde_json::{Value, json, to_value};
use shed_proto::{
    Color, Line, Method, Run, ScreenUpdate, TerminalParams, TerminalResizeParams, WatchParams,
};

pub(crate) fn screen_updates() -> [ScreenUpdate; 2] {
    let plain = |text: &str| Run {
        text: text.into(),
        fg: None,
        bg: None,
        bold: false,
        italic: false,
        underline: false,
        inverse: false,
    };
    let ok = Run {
        fg: Some(Color::Index(2)),
        bold: true,
        ..plain("ok")
    };
    let first = ScreenUpdate {
        seq: 41,
        full: true,
        rows: 2,
        cols: 40,
        cursor: [1, 2],
        scrollback: 120,
        lines: vec![
            Line {
                row: 0,
                runs: vec![ok, plain(" 12 tests")],
            },
            Line {
                row: 1,
                runs: vec![plain("$")],
            },
        ],
    };
    let typed = Run {
        bg: Some(Color::Rgb([40, 40, 40])),
        ..plain("make")
    };
    let next = ScreenUpdate {
        seq: 44,
        full: false,
        cursor: [1, 6],
        lines: vec![Line {
            row: 1,
            runs: vec![plain("$ "), typed],
        }],
        ..first.clone()
    };
    [first, next]
}

pub(crate) fn example(method: Method) -> Option<(Value, Value)> {
    let params = match method {
        Method::ScreenWatch => to_value(WatchParams {
            terminal: "build".into(),
            since: Some(41),
        }),
        Method::Input => Ok(json!({"terminal": "build", "text": "make lint"})),
        Method::TerminalKeys => {
            Ok(json!({"terminal": "build", "keys": ["esc", "ctrl-c", "up", "enter"]}))
        }
        Method::TerminalResize => to_value(TerminalResizeParams {
            terminal: None,
            cols: 60,
            rows: 30,
        }),
        Method::ReadOnly => to_value(TerminalParams {
            terminal: "agent".into(),
        }),
        _ => return None,
    };
    Some((params.unwrap(), json!({})))
}
