//! Programs run on the machine with no runtime and no clock: values, closures, destructuring, errors, limits.
#![cfg(test)]

use serde_json::{Value as Json, json};
use shed_script::{Limits, Program, Run, ScriptError, Step};

const SLICE: u64 = 10_000;

fn finish(run: &mut Run) -> Result<Json, ScriptError> {
    loop {
        let before = run.counts().steps;
        match run.step() {
            Step::More => assert!(run.counts().steps - before <= SLICE),
            Step::Done(outcome) => return outcome,
        }
    }
}

fn run_with(code: &str, limits: Limits) -> (Result<Json, ScriptError>, Run) {
    let program = Program::parse(code).unwrap();
    let mut run = Run::new(&program, limits);
    (finish(&mut run), run)
}

fn outcome(code: &str) -> Result<Json, ScriptError> {
    Program::parse(code).and_then(|program| finish(&mut Run::new(&program, Limits::default())))
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
    let mut run = Run::new(&program, Limits::default());
    assert_eq!(finish(&mut run).unwrap(), json!(200));
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
