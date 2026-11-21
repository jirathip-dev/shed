//! The globals, the methods of strings and arrays, `JSON` and `console`, run on the machine.
#![cfg(test)]

mod common;

use serde_json::{Value as Json, json};
use shed_script::vm::{Limits, Run};
use shed_script::{Program, ScriptError};

use common::{Fixture, drive};

fn run_with(code: &str, limits: Limits) -> (Result<Json, ScriptError>, Run<'static, Fixture>) {
    let program = Program::parse(code).unwrap();
    let host: &'static Fixture = Box::leak(Box::default());
    let mut run = Run::new(&program, limits, host);
    (drive(&mut run, host), run)
}

fn run(code: &str) -> Json {
    run_with(code, Limits::default()).0.unwrap()
}

fn fails(code: &str) -> String {
    run_with(code, Limits::default()).0.unwrap_err().to_string()
}

const LISTED: &[&str] = &[
    "console",
    "JSON",
    "Math",
    "Object",
    "Array",
    "String",
    "Number",
    "Boolean",
    "Error",
    "parseInt",
    "parseFloat",
    "isNaN",
    "NaN",
    "Infinity",
    "undefined",
];

const NOT_THERE: &[&str] = &[
    "require",
    "process",
    "fetch",
    "globalThis",
    "Date",
    "setTimeout",
    "setInterval",
    "eval",
    "Function",
    "Symbol",
    "Reflect",
    "Proxy",
    "Buffer",
    "RegExp",
    "Map",
    "Set",
    "WeakMap",
    "WeakRef",
    "Intl",
    "BigInt",
    "ArrayBuffer",
    "Uint8Array",
    "DataView",
    "Atomics",
    "SharedArrayBuffer",
    "WebAssembly",
    "isFinite",
    "encodeURI",
    "decodeURIComponent",
    "escape",
    "queueMicrotask",
    "structuredClone",
    "crypto",
    "performance",
    "navigator",
    "window",
    "self",
    "global",
    "module",
    "exports",
    "__dirname",
    "Deno",
    "Bun",
    "TypeError",
    "arguments",
];

#[test]
fn only_the_listed_globals_exist() {
    for name in LISTED {
        let kind = run(&format!("return [typeof {name}, {name} === undefined]"));
        let missing = json!(["undefined", true]);
        assert_eq!(kind == missing, *name == "undefined", "{name}: {kind}");
        if !matches!(*name, "NaN" | "Infinity" | "undefined") {
            assert_eq!(
                fails(&format!("{name} = 1")),
                format!(
                    "TypeError 1:{}: {name} is a constant: declare it with let to assign to it",
                    name.len() + 2
                )
            );
        }
    }
    for name in NOT_THERE {
        assert_eq!(run(&format!("return typeof {name}")), json!("undefined"));
        let error = fails(&format!("return {name}"));
        assert!(
            error.starts_with(&format!("ReferenceError 1:8: {name} is not defined")),
            "{name}: {error}"
        );
    }
    assert_eq!(
        fails("return require('fs')"),
        "ReferenceError 1:8: require is not defined: a script reaches the outside only through shed.<method>(params)"
    );
    assert_eq!(
        run(
            "return [typeof console, typeof JSON, typeof Math, typeof Object, typeof String.fromBytes, typeof Math.random, typeof 'a'.slice, typeof [].map]"
        ),
        json!([
            "object",
            "object",
            "object",
            "function",
            "function",
            "undefined",
            "function",
            "function"
        ])
    );
}

macro_rules! returns {
    ($($test:ident: $code:expr => $value:expr;)*) => {$(
        #[test]
        fn $test() {
            assert_eq!(run($code), $value);
        }
    )*};
}

returns! {
    string_methods: "const s = '  Héllo, world  '.trim(); return [s.length, s.at(-1), s.charAt(1), s.slice(1, 4), s.slice(-5), s.substring(4, 1), s.split(', '), 'a,b,c'.split(',', 2), 'ab'.split(''), s.startsWith('Hé'), s.endsWith('world'), s.includes('lo'), s.indexOf('o'), s.lastIndexOf('o'), s.indexOf('z'), s.toUpperCase(), s.toLowerCase()]" => json!([12, "d", "é", "éll", "world", "éll", ["Héllo", "world"], ["a", "b"], ["a", "b"], true, true, true, 4, 8, -1, "HÉLLO, WORLD", "héllo, world"]);
    string_edits: "return ['a-b-c'.replace('-', '+'), 'a-b-c'.replaceAll('-', ''), '7'.padStart(3, '0'), 'x'.padEnd(3), 'ab'.repeat(3), 'a'.concat(1, 'b', null), 'b'.localeCompare('a'), '\\tx\\n'.trimStart(), ' x '.trimEnd()]" => json!(["a+b-c", "abc", "007", "x  ", "ababab", "a1bnull", 1, "x\n", " x"]);
    strings_and_bytes: "const bytes = 'hé'.toBytes(); return [bytes, String.fromBytes(bytes), String.fromBytes([104, 105])]" => json!([[104, 195, 169], "hé", "hi"]);
    matches_takes_a_wait_pattern: "return ['error: disk full'.matches('^error: .*full$'), 'ok'.matches('err'), 'a1'.matches('[0-9]')]" => json!([true, false, true]);
    array_methods: "const a = [3, 1, 2]; const n = a.push(4, 5); const last = a.pop(); const first = a.shift(); a.unshift(0); return [n, last, first, a, a.at(-1), a.slice(1, 3), a.concat([9], 8), a.join('-'), [1, [2, [3, [4]]]].flat(), [1, [2, [3]]].flat(2), a.includes(2), [NaN].includes(NaN), [NaN].indexOf(NaN), [1, 2, 1].lastIndexOf(1), a.indexOf(7), ['a', 'b'].entries(), [1, 2].reverse(), [null, undefined, 1].join()]" => json!([5, 5, 3, [0, 1, 2, 4], 4, [1, 2], [0, 1, 2, 4, 9, 8], "0-1-2-4", [1, 2, [3, [4]]], [1, 2, 3], true, true, -1, 2, -1, [[0, "a"], [1, "b"]], [2, 1], ",,1"]);
    callbacks_run_in_the_prelude: "const xs = [5, 3, 8, 1]; return [xs.map((x, i) => x * i), xs.filter(x => x > 2), xs.find(x => x > 5), xs.findIndex(x => x === 3), xs.some(x => x > 7), xs.every(x => x > 0), xs.reduce((a, b) => a + b), xs.reduce((a, b) => a + b, 10), [[1], [2, 3]].flatMap(x => x), [...xs].sort(), xs.sort((a, b) => b - a), xs]" => json!([[0, 3, 16, 3], [5, 3, 8], 8, 1, true, true, 17, 27, [1, 2, 3], [1, 3, 5, 8], [8, 5, 3, 1], [8, 5, 3, 1]]);
    sort_is_stable: "const xs = [{ k: 1, n: 'a' }, { k: 0, n: 'b' }, { k: 1, n: 'c' }, { k: 0, n: 'd' }]; let seen = 0; xs.forEach(() => { seen++ }); return [xs.sort((a, b) => a.k - b.k).map(x => x.n).join(''), ['b', 'a', 'c'].sort(), seen]" => json!(["bdac", ["a", "b", "c"], 4]);
    objects: "const o = { a: 1, b: 2 }; return [Object.keys(o), Object.values(o), Object.entries(o), Object.fromEntries([['x', 1], ['y', 2]]), Object.assign(o, { c: 3 }, null), Object.hasOwn(o, 'c'), Object.hasOwn(o, 'z'), Object.keys(['p', 'q']), Array.isArray([]), Array.isArray('a'), Array.from('ab'), Array.from([1, 2])]" => json!([["a", "b"], [1, 2], [["a", 1], ["b", 2]], {"x": 1, "y": 2}, {"a": 1, "b": 2, "c": 3}, true, false, ["0", "1"], true, false, ["a", "b"], [1, 2]]);
    numbers: "return [Number('12.5'), Number(''), Number('0x1f'), Number('1e3'), isNaN(Number('abc')), Number(true), Number(null), Number.isInteger(5), Number.isInteger(5.5), Number.isFinite(1 / 0), parseInt('42px'), parseInt('ff', 16), parseInt('0x10'), parseInt('-7.9'), isNaN(parseInt('x')), parseFloat('2.5abc'), parseFloat('.5e1'), parseFloat('-Infinity') < 0, String(12), String([1, 'a']), String(null), Boolean(''), Boolean('x'), Error('m').message]" => json!([12.5, 0, 31, 1000, true, 1, 0, true, false, false, 42, 255, 16, -7, true, 2.5, 5, true, "12", "[1,\"a\"]", "null", false, true, "m"]);
    math: "return [Math.min(3, 1, 2), Math.max() === -Infinity, Math.floor(-1.5), Math.ceil(1.2), Math.round(2.5), Math.round(-2.5), Math.trunc(-1.7), Math.abs(-3), Math.sign(-2), Math.sqrt(16), Math.pow(2, 10), 1 / Math.round(-0.4) === -Infinity, isNaN(Math.max(1, NaN))]" => json!([1, true, -2, 2, 3, -2, -1, 3, -1, 4, 1024, true, true]);
    json_both_ways: "const v = JSON.parse('{\"b\": [1, 2.5, \"x\", null, true], \"a\": {}}'); return [v, JSON.stringify(v), JSON.stringify({ a: [1, { b: 2 }], u: undefined, f: () => 1, n: NaN }, null, 2), JSON.stringify(undefined), JSON.stringify('q\"'), JSON.stringify([undefined, 1e21])]" => json!([{"b": [1, 2.5, "x", null, true], "a": {}}, "{\"b\":[1,2.5,\"x\",null,true],\"a\":{}}", "{\n  \"a\": [\n    1,\n    {\n      \"b\": 2\n    }\n  ],\n  \"n\": null\n}", null, "\"q\\\"\"", "[null,1e+21]"]);
}

#[test]
fn infinities_reach_json_as_errors() {
    assert_eq!(
        fails("return Math.max()"),
        "TypeError 1:1: result is -Infinity: JSON has no such number"
    );
}

macro_rules! throws {
    ($($test:ident: $code:expr => $error:expr;)*) => {$(
        #[test]
        fn $test() {
            assert_eq!(fails($code), $error);
        }
    )*};
}

throws! {
    no_implicit_conversion_in_math: "return Math.floor('2')" => "TypeError 1:13: Math.floor takes a number, not a string: convert with Number(x)";
    replace_takes_a_string: "return 'a'.replace(1, 'b')" => "TypeError 1:12: .replace (a string, not a regex) takes a string, not a number";
    a_bad_pattern_is_a_syntax_error: "return 'a'.matches('(')" => "SyntaxError 1:12: bad pattern \"(\": unclosed (";
    bad_json_is_a_syntax_error: "return JSON.parse('{')" => "SyntaxError 1:13: JSON.parse: EOF while parsing an object at line 1 column 1";
    a_replacer_is_refused: "return JSON.stringify({}, (k, v) => v)" => "TypeError 1:13: JSON.stringify takes no replacer: pass null, and map the value first";
    a_negative_repeat_is_a_range_error: "return 'a'.repeat(-1)" => "RangeError 1:12: repeat(-1): give a count from 0";
    an_error_in_a_callback_is_at_its_line: "const xs = [1, 2]\nreturn xs.map((x) => {\n  if (x === 2) throw new Error('two')\n  return x\n})" => "Error 3:16: two";
    a_prelude_error_is_at_the_call: "const xs = []\nreturn xs.reduce((a, b) => a + b)" => "TypeError 2:11: reduce of an empty array needs a start value";
    a_callback_that_is_none: "return [1].map(5)" => "TypeError 1:12: number is not a function";
    a_namespace_is_not_called: "return Math(1)" => "TypeError 1:8: Math is not a function";
}

#[test]
fn errors_from_builtins_are_caught() {
    assert_eq!(
        run("try { JSON.parse('nope') } catch (e) { return [e.name, e.line] }"),
        json!(["SyntaxError", 1])
    );
}

#[test]
fn memory_is_reserved_before_a_builtin_allocates() {
    let limits = Limits {
        memory: 1 << 20,
        ..Limits::default()
    };
    for code in [
        "return 'ab'.repeat(1e9)",
        "return 'x'.padStart(1e9)",
        "const s = 'x'.repeat(100000); return s.toBytes()",
        "const s = 'x'.repeat(400000); return [s, s, s].join('')",
    ] {
        let (outcome, run) = run_with(code, limits);
        assert_eq!(
            outcome.unwrap_err().code.as_deref(),
            Some("MEMORY"),
            "{code}"
        );
        assert!(run.counts().peak_bytes <= 1 << 20, "{code}");
    }
}

#[test]
fn builtins_pay_fuel_for_what_they_touch() {
    let (outcome, short) = run_with("const s = 'x'.repeat(64)\nreturn 1", Limits::default());
    outcome.unwrap();
    let (outcome, long) = run_with("const s = 'x'.repeat(640000)\nreturn 1", Limits::default());
    outcome.unwrap();
    assert_eq!(long.counts().steps - short.counts().steps, 9999);
    let limits = Limits {
        fuel: 1000,
        ..Limits::default()
    };
    let (outcome, run) = run_with("const s = 'x'.repeat(1e6)\nreturn s.length", limits);
    assert_eq!(outcome.unwrap_err().code.as_deref(), Some("FUEL"));
    assert_eq!(run.counts().steps, 1000);
}

#[test]
fn console_lines_are_logs() {
    let (outcome, run) = run_with(
        "console.log('a', 1, [2], { b: null }); console.error('e'); console.debug(); return 1",
        Limits::default(),
    );
    outcome.unwrap();
    assert_eq!(run.logs(), ["a 1 [2] {\"b\":null}", "e", ""]);
}

#[test]
fn logs_stop_at_their_limit() {
    let limits = Limits {
        logs: 100,
        ..Limits::default()
    };
    let (outcome, run) = run_with(
        "for (let i = 0; i < 100; i++) { console.log('0123456789') }",
        limits,
    );
    let error = outcome.unwrap_err();
    assert_eq!(error.code.as_deref(), Some("LOGS"));
    assert_eq!(run.logs().len(), 10);
}
