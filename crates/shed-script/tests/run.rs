//! Programs run on the machine with no runtime and no clock: values, closures, destructuring, errors, limits.
#![cfg(test)]

mod common;

use serde_json::{Value as Json, json};
use shed_script::vm::{Limit, Limits, Run, Step};
use shed_script::{HostError, Program, ScriptError};

use common::{Answer, Fixture, drive};

fn run_on(
    code: &str,
    limits: Limits,
    answers: Vec<Answer>,
) -> (
    Result<Json, ScriptError>,
    &'static Fixture,
    Run<'static, Fixture>,
) {
    let program = Program::parse(code).unwrap();
    let host: &'static Fixture = Box::leak(Box::new(Fixture {
        answers,
        ..Fixture::default()
    }));
    let mut run = Run::new(&program, limits, host);
    (drive(&mut run, host), host, run)
}

fn run_with(code: &str, limits: Limits) -> (Result<Json, ScriptError>, Run<'static, Fixture>) {
    let (outcome, _, run) = run_on(code, limits, Vec::new());
    (outcome, run)
}

fn outcome(code: &str) -> Result<Json, ScriptError> {
    Program::parse(code)?;
    run_with(code, Limits::default()).0
}

fn run(code: &str) -> Json {
    outcome(code).unwrap()
}

fn fails(code: &str) -> String {
    outcome(code).unwrap_err().to_string()
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
    arithmetic: "return [1 + 2 * 3, 7 % 4, 2 ** 10, 1 / 2, -(3), 10 - 2 - 3]" => json!([7, 3, 1024, 0.5, -3, 5]);
    whole_numbers_reach_json_as_integers: "return { lines: 15, big: 2 ** 53, half: 1.5 }" => json!({"lines": 15, "big": 9_007_199_254_740_992_i64, "half": 1.5});
    strings_join_and_compare: "return ['a' + 1, 'b' < 'c', 'x' === 'x', `n=${1 + 1} ${[1, 'a']} ${null}`]" => json!(["a1", true, true, "n=2 [1,\"a\"] null"]);
    numbers_as_text: "return `${0.1 + 0.2} ${1e21} ${1e-7} ${-0} ${123.5} ${NaN} ${-Infinity}`" => json!("0.30000000000000004 1e+21 1e-7 0 123.5 NaN -Infinity");
    logic_short_circuits: "let n = 0; const f = () => { n++; return 1 }; const a = false && f(); const b = null ?? 2; const c = 0 || 'x'; return [a, b, c, n]" => json!([false, 2, "x", 0]);
    optional_chains_stop_at_null: "const o = { a: null, b: { c: 1 } }; return [o.a?.b.c, o.b?.c, o.z?.(1), o.a == null, o.b != null]" => json!([null, 1, null, true, true]);
    typeof_names_kinds: "return [typeof 1, typeof 'a', typeof null, typeof [], typeof {}, typeof (() => 1), typeof nope, typeof undefined]" => json!(["number", "string", "object", "object", "object", "function", "undefined", "undefined"]);
    compound_assignment: "let a = 1; a += 2; a *= 3; let o = { n: 1, s: null }; o.n -= 5; o.s ??= 'set'; o.n ||= 9; const x = [1]; x[0] += 1; return [a, o, x]" => json!([9, {"n": -4, "s": "set"}, [2]]);
    updates_return_old_and_new: "let i = 5; const a = i++; const b = ++i; const o = { n: 1 }; const c = o.n++; const d = --o.n; const e = [3]; const f = e[0]--; return [a, b, i, c, d, f, e]" => json!([5, 7, 7, 1, 1, 3, [2]]);
    loops_break_and_continue: "let out = []; for (let i = 0; i < 10; i++) { if (i % 2) continue; if (i > 6) break; out[out.length] = i } let j = 0; while (true) { j++; if (j === 3) break } return [out, j]" => json!([[0, 2, 4, 6], 3]);
    for_of_walks_arrays_and_strings: "const out = []; for (const x of [1, 2, 3]) { if (x === 2) continue; out[out.length] = x } for (const c of 'hé') { out[out.length] = c } return out" => json!([1, 3, "h", "é"]);
    closures_keep_their_scope: "const counter = () => { let n = 0; return () => ++n }; const a = counter(); const b = counter(); a(); a(); return [a(), b()]" => json!([3, 1]);
    each_iteration_has_its_own_binding: "const fs = []; for (let i = 0; i < 3; i++) { fs[fs.length] = () => i } for (const x of ['a', 'b']) { fs[fs.length] = () => x } return [fs[0](), fs[1](), fs[2](), fs[3](), fs[4]()]" => json!([0, 1, 2, "a", "b"]);
    functions_are_hoisted: "return twice(4); function twice(n) { return n * 2 }" => json!(8);
    recursion_works: "function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2) } return fib(15)" => json!(610);
    defaults_and_rest_parameters: "const f = (a, b = a + 1, ...rest) => [a, b, rest]; return [f(1), f(1, 5, 6, 7), f(0)]" => json!([[1, 2, []], [1, 5, [6, 7]], [0, 1, []]]);
    destructuring_objects: "const k = 'c'; const { a, b: { x = 9 } = {}, [k]: c, ...rest } = { a: 1, c: 3, d: 4, e: 5 }; return [a, x, c, rest]" => json!([1, 9, 3, {"d": 4, "e": 5}]);
    destructuring_arrays: "const [a, , b = 2, ...rest] = [1, 0, undefined, 4, 5]; const [[c], { d }] = [[3], { d: 4 }]; return [a, b, c, d, rest]" => json!([1, 2, 3, 4, [4, 5]]);
    destructuring_parameters: "const f = ({ name, kind = 'none' }, [first]) => `${name}:${kind}:${first}`; return f({ name: 'a' }, ['z'])" => json!("a:none:z");
    spreads_in_literals_and_calls: "const f = (...xs) => xs; const a = [1, 2]; return [[0, ...a, 3], { ...{ x: 1 }, y: 2, ['z' + 1]: 3 }, f(...a, ...'ab')]" => json!([[0, 1, 2, 3], {"x": 1, "y": 2, "z1": 3}, [1, 2, "a", "b"]]);
    try_catches_a_throw: "try { throw new Error('boom') } catch (e) { return [e.name, e.message, e.line] }" => json!(["Error", "boom", 1]);
    try_catches_a_type_error: "try { const o = null; o.x } catch ({ name, message }) { return [name, message] }" => json!(["TypeError", "cannot read .x of null: guard it with ?."]);
    try_catches_across_frames: "const deep = (n) => { if (n === 0) throw { code: 7 }; return deep(n - 1) }; let r; try { deep(50) } catch (e) { r = e.code } return [r, deep.length]" => json!([7, null]);
    catch_without_a_binding_and_after: "let seen = 'no'; try { undefinedName } catch { seen = 'yes' } return seen" => json!("yes");
    break_out_of_try_in_a_loop: "let n = 0; for (const x of [1, 2, 3]) { try { if (x === 2) break; n += x } catch (e) {} } try { throw 1 } catch (e) { n += 10 } return n" => json!(11);
    arrays_append_at_their_length: "const a = []; for (let i = 0; i < 5; i++) { a[i] = i * i } return [a, a.length, 'héllo'.length, 'abc'[1]]" => json!([[0, 1, 4, 9, 16], 5, 5, "b"]);
    nothing_returned_is_null: "const a = 1" => json!(null);
    undefined_fields_are_left_out: "return { a: undefined, b: [undefined] }" => json!({"b": [null]});
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
    an_unknown_name_is_a_reference_error: "const a = 1\nreturn b + a" => "ReferenceError 2:8: b is not defined";
    no_implicit_conversion: "return '5' * 2" => "TypeError 1:12: * between a string and a number: no implicit conversion here, use String(x) or Number(x)";
    a_constant_cannot_be_assigned: "const a = 1; a = 2" => "TypeError 1:16: a is a constant: declare it with let to assign to it";
    calling_a_non_function: "const o = {}; o.f(1)" => "TypeError 1:17: undefined is not a function";
    an_uncaught_throw_ends_the_run: "function f() {\n  throw new Error('no ' + 'way')\n}\nf()" => "Error 2:3: no way";
    a_thrown_string_is_its_message: "throw 'plain'" => "Error 1:1: plain";
    a_hole_is_a_range_error: "const a = []; a[3] = 1" => "RangeError 1:20: index 3 of an array of 0: set one that exists, or the one at its length";
    a_function_is_no_json: "return { f: () => 1 }" => "TypeError 1:1: result.f is a function: JSON cannot carry it";
    declared_twice_is_a_syntax_error: "let a = 1\nlet a = 2" => "SyntaxError 2:5: a is declared twice in one block";
}

#[test]
fn a_value_that_holds_itself_is_no_json() {
    let error = fails("const a = [1]; a[1] = a; return a");
    assert!(error.starts_with("TypeError 1:26: result[1][1]"), "{error}");
    assert!(
        error.ends_with("is nested over 200 deep (does it hold itself?)"),
        "{error}"
    );
}

#[test]
fn a_busy_loop_stops_on_fuel() {
    let (outcome, run) = run_with("let n = 0\nwhile (true) {}", Limits::default());
    let error = outcome.unwrap_err();
    assert_eq!(error.code.as_deref(), Some("FUEL"));
    assert_eq!(error.name, "LimitError");
    assert_eq!(run.counts().steps, 50_000_000);
}

#[test]
fn fuel_cannot_be_caught() {
    let limits = Limits {
        fuel: 100_000,
        ..Limits::default()
    };
    let (outcome, run) = run_with("try { for (;;) {} } catch (e) { return 'caught' }", limits);
    assert_eq!(outcome.unwrap_err().code.as_deref(), Some("FUEL"));
    assert_eq!(run.counts().steps, 100_000);
}

#[test]
fn deep_recursion_stops_on_depth() {
    let code = "let deepest = 0\nconst down = (n) => { deepest = n; return down(n + 1) }\ntry { down(2) } catch (e) { return 'caught' }";
    let (outcome, _) = run_with(code, Limits::default());
    let error = outcome.unwrap_err();
    assert_eq!(error.code.as_deref(), Some("DEPTH"));
    assert_eq!((error.line, error.column), (2, 43));
    let program =
        Program::parse("const down = (n) => n === 200 ? n : down(n + 1)\nreturn down(2)").unwrap();
    let host = Fixture::default();
    let mut run = Run::new(&program, Limits::default(), &host);
    assert_eq!(drive(&mut run, &host).unwrap(), json!(200));
}

#[test]
fn growing_values_stop_on_memory() {
    let limits = Limits {
        memory: 4 << 20,
        ..Limits::default()
    };
    let (outcome, run) = run_with(
        "const a = []\nwhile (true) { a[a.length] = { n: a.length } }",
        limits,
    );
    assert_eq!(outcome.unwrap_err().code.as_deref(), Some("MEMORY"));
    assert!(run.counts().peak_bytes <= 4 << 20);
    assert!(run.counts().steps < 50_000_000);
}

#[test]
fn garbage_is_not_counted() {
    let limits = Limits {
        memory: 4 << 20,
        ..Limits::default()
    };
    let code = "let s = ''\nfor (let i = 0; i < 20000; i++) { s = `${i}`.padEnd ?? 'x'; const big = [s, s, s, s, s, s, s, s, s, s]; const o = { big, more: [...big, ...big, ...big] } }\nreturn 'done'";
    let (outcome, run) = run_with(code, limits);
    assert_eq!(outcome.unwrap(), json!("done"));
    assert!(run.counts().collections > 0, "{:?}", run.counts());
}

fn ok(at: u64) -> Answer {
    (at, Ok(json!({"name": "t"})))
}

fn refused(at: u64, code: &str) -> Answer {
    (
        at,
        Err(HostError {
            name: "ShedError".into(),
            code: code.into(),
            message: format!("read: {code}: impl-19"),
        }),
    )
}

returns! {
    await_takes_a_value_or_a_promise: "const f = async (x) => x * 2; const a = await f(2); const b = await 3; const c = await (async () => f(5))(); return [a, b, c]" => json!([4, 3, 10]);
    an_async_call_runs_until_its_first_await: "const out = []; const t = async (n) => { out.push(n); await null; out.push(n + 10) }; const p = t(1); out.push(0); await p; return out" => json!([1, 0, 11]);
    a_rejection_is_caught_at_its_await: "const g = async () => { throw new Error('no') }; try { await g() } catch (e) { return e.message }" => json!("no");
    promise_helpers: "const f = async (x) => x; const g = async () => { throw new Error('no') }; const all = await Promise.all([f(1), 2, Promise.resolve(3)]); const settled = await Promise.allSettled([g(), f(4)]); const race = await Promise.race([f(7), f(8)]); const then = await f(1).then((v) => v + 1); const caught = await g().catch((e) => e.message); let rejected; try { await Promise.reject('r') } catch (e) { rejected = e } return [all, settled.map((s) => s.status), settled[1].value, race, then, caught, rejected, await Promise.all([])]" => json!([[1, 2, 3], ["rejected", "fulfilled"], 4, 7, 2, "no", "r", []]);
}

throws! {
    a_promise_is_no_json: "const f = async () => 1; return { p: f() }" => "TypeError 1:26: result.p is a Promise: await it (Promise.all for a list)";
    an_uncaught_rejection_ends_the_run: "const g = async () => { throw new Error('deep') }\nawait g()" => "Error 2:1: deep";
}

#[test]
fn promise_all_runs_its_calls_at_the_same_time() {
    let code = "return Promise.all([shed.read({ name: 'a' }), shed.read({ name: 'b' })])";
    let (outcome, host, _) = run_on(code, Limits::default(), vec![ok(10)]);
    assert_eq!(outcome.unwrap(), json!([{"name": "t"}, {"name": "t"}]));
    let state = host.state.lock().unwrap();
    assert_eq!((state.most_open, state.now), (2, 10));
    let code = "const xs = []\nfor (let i = 0; i < 40; i++) xs.push(shed.read({ name: `t${i}` }))\nreturn (await Promise.all(xs)).length";
    let (outcome, host, _) = run_on(code, Limits::default(), vec![ok(10)]);
    assert_eq!(outcome.unwrap(), json!(40));
    let state = host.state.lock().unwrap();
    assert_eq!((state.most_open, state.calls.len()), (32, 40));
    assert_eq!(state.calls[39], ("read".into(), json!({"name": "t39"})));
}

#[test]
fn promise_all_rejects_at_the_first_rejection() {
    let code = "return Promise.all([shed.wait({}), shed.read({})])";
    let (outcome, host, _) = run_on(
        code,
        Limits::default(),
        vec![ok(100), refused(5, "NO_SUCH_TERMINAL")],
    );
    let error = outcome.unwrap_err();
    assert_eq!(
        error.to_string(),
        "ShedError 1:41: read: NO_SUCH_TERMINAL: impl-19"
    );
    assert_eq!(
        (error.code.as_deref(), error.method.as_deref()),
        (Some("NO_SUCH_TERMINAL"), Some("read"))
    );
    assert_eq!(host.state.lock().unwrap().now, 5);
}

#[test]
fn a_shed_error_is_caught_with_its_code() {
    let code = "try {\n  await shed.file.read({ path: 'x' })\n} catch (e) {\n  return [e.name, e.code, e.method, e.line, e.column]\n}";
    let (outcome, host, _) = run_on(code, Limits::default(), vec![refused(1, "NO_SUCH_FILE")]);
    assert_eq!(
        outcome.unwrap(),
        json!(["ShedError", "NO_SUCH_FILE", "file.read", 2, 19])
    );
    assert_eq!(host.state.lock().unwrap().calls[0].0, "file.read");
}

#[test]
fn the_2001st_call_stops_the_run() {
    let code = "const xs = []\nfor (let i = 0; i < 2000; i++) xs.push(shed.get({}))\nreturn (await Promise.all(xs)).length";
    assert_eq!(
        run_on(code, Limits::default(), vec![ok(0)]).0.unwrap(),
        json!(2000)
    );
    let code = "for (let i = 0; i < 2001; i++) shed.get({})";
    let (outcome, _, run) = run_on(code, Limits::default(), vec![refused(0, "NOT_IN_SCRIPT")]);
    let error = outcome.unwrap_err();
    assert_eq!(
        (error.name.as_str(), error.code.as_deref()),
        ("LimitError", Some("CALLS"))
    );
    assert_eq!(run.counts().calls, 2000);
}

#[test]
fn an_unawaited_send_is_sent() {
    let code = "shed.send({ name: 'a', text: 'go' })\nreturn 'sent'";
    let (outcome, host, _) = run_on(code, Limits::default(), vec![ok(7)]);
    assert_eq!(outcome.unwrap(), json!("sent"));
    let state = host.state.lock().unwrap();
    assert_eq!((state.calls.len(), state.open.len(), state.now), (1, 0, 7));
    let (outcome, _, _) = run_on(
        code,
        Limits::default(),
        vec![refused(7, "NO_SUCH_TERMINAL")],
    );
    assert_eq!(
        outcome.unwrap_err().to_string(),
        "ShedError 1:6: read: NO_SUCH_TERMINAL: impl-19"
    );
}

#[test]
fn the_deadline_names_the_calls_in_flight() {
    let program =
        Program::parse("const a = 1\nconst got = await shed.wait({ name: 'a' })\nreturn got")
            .unwrap();
    let host = Fixture {
        answers: vec![ok(60_000)],
        ..Fixture::default()
    };
    let mut run = Run::new(&program, Limits::default(), &host);
    assert_eq!(run.step(), Step::Waiting);
    let error = run.stopped(Limit::Deadline);
    assert_eq!(
        error.to_string(),
        "LimitError 2:13: stopped at the deadline; in flight: wait"
    );
    assert_eq!(error.code.as_deref(), Some("DEADLINE"));
    assert_eq!(run.step(), Step::Done(Err(error)));
}

#[test]
fn async_recursion_stops_on_memory() {
    let limits = Limits {
        memory: 4 << 20,
        ..Limits::default()
    };
    let code = "const down = async (n) => n === 0 ? 0 : 1 + (await down(n - 1))\nreturn await down(1000000)";
    let (outcome, run) = run_with(code, limits);
    assert_eq!(outcome.unwrap_err().code.as_deref(), Some("MEMORY"));
    assert!(run.counts().peak_bytes <= 4 << 20);
}

#[test]
fn a_run_is_send() {
    fn send<T: Send>(_: &T) {}
    let program = Program::parse("return 1").unwrap();
    let host = Fixture::default();
    send(&Run::new(&program, Limits::default(), &host));
}
