//! Every construct left out of the subset is a `SyntaxError` naming it, at its line and column.
#![cfg(test)]

use shed_script::ast::{Expr, Item, Stmt};
use shed_script::parse;

fn refused(code: &str) -> String {
    let error = parse(code).expect_err(code);
    assert_eq!(error.name, "SyntaxError");
    error.to_string()
}

macro_rules! out {
    ($($test:ident: $code:expr => $error:expr;)*) => {$(
        #[test]
        fn $test() {
            assert_eq!(refused($code), $error);
        }
    )*};
}

out! {
    var_is_out: "let a = 1\nvar b = 2" => "SyntaxError 2:1: var is not in the subset: use let, or const";
    generator_is_out: "function* gen() {}" => "SyntaxError 1:1: a generator (function*) is not in the subset: return an array";
    generator_method_is_out: "const o = { *gen() {} }" => "SyntaxError 1:13: a generator (function*) is not in the subset: return an array";
    yield_is_out: "const f = () => { yield 1 }" => "SyntaxError 1:19: yield is not in the subset: there are no generators, return an array";
    this_is_out: "return this.x" => "SyntaxError 1:8: this is not in the subset: pass the value as a parameter";
    class_is_out: "class A {}" => "SyntaxError 1:1: class is not in the subset: use an object of functions";
    getter_is_out: "const o = { get x() { return 1 } }" => "SyntaxError 1:13: a getter (get x() {}) is not in the subset: use a plain property";
    setter_is_out: "const o = { set x(v) {} }" => "SyntaxError 1:13: a setter (set x(v) {}) is not in the subset: assign the property";
    new_is_out: "const m = new Map()" => "SyntaxError 1:11: new is not in the subset but for new Error(message): use a function that returns an object";
    switch_is_out: "switch (x) { case 1: break }" => "SyntaxError 1:1: switch is not in the subset: use if and else if";
    do_is_out: "do { x++ } while (x < 3)" => "SyntaxError 1:1: do … while is not in the subset: use while";
    for_in_is_out: "for (const k in o) {}" => "SyntaxError 1:14: for … in is not in the subset: use for (const key of Object.keys(object))";
    for_in_without_decl_is_out: "for (k in o) {}" => "SyntaxError 1:6: for … in is not in the subset: use for (const key of Object.keys(object))";
    finally_is_out: "try { a() } catch (e) { b() } finally { c() }" => "SyntaxError 1:31: finally is not in the subset: run the cleanup after the try";
    finally_without_catch_is_out: "try { a() } finally { c() }" => "SyntaxError 1:13: finally is not in the subset: run the cleanup after the try";
    label_is_out: "outer: for (;;) {}" => "SyntaxError 1:1: a label is not in the subset: use a flag, or return from a function";
    labelled_break_is_out: "for (;;) { break outer }" => "SyntaxError 1:18: a label is not in the subset: use a flag, or return from a function";
    with_is_out: "with (o) { a }" => "SyntaxError 1:1: with is not in the subset: name the object";
    import_is_out: "import fs from \"fs\"" => "SyntaxError 1:1: import is not in the subset: a script has only its globals";
    dynamic_import_is_out: "const m = await import(\"fs\")" => "SyntaxError 1:17: import is not in the subset: a script has only its globals";
    export_is_out: "export const a = 1" => "SyntaxError 1:1: export is not in the subset: return the value";
    regex_is_out: "const a = 1\nconst b = 2\nif (x.match(/ab+c/)) {}" => "SyntaxError 3:13: a regex literal is not in the subset: use text.includes(…) or text.matches(pattern)";
    regex_with_a_quote_is_out: "const r = /it's/" => "SyntaxError 1:11: a regex literal is not in the subset: use text.includes(…) or text.matches(pattern)";
    bigint_is_out: "const n = 10n" => "SyntaxError 1:11: a BigInt literal (1n) is not in the subset: use a number";
    tagged_template_is_out: "const s = tag`a${b}`" => "SyntaxError 1:14: a tagged template is not in the subset: call the function with a template string";
    loose_eq_is_out: "if (a == 1) {}" => "SyntaxError 1:7: == is only for null in the subset: use === (or == null)";
    loose_ne_is_out: "if (a != b) {}" => "SyntaxError 1:7: != is only for null in the subset: use !== (or != null)";
    in_is_out: "if (\"a\" in o) {}" => "SyntaxError 1:9: the in operator is not in the subset: use Object.hasOwn(object, key)";
    instanceof_is_out: "if (e instanceof Error) {}" => "SyntaxError 1:7: instanceof is not in the subset: use typeof or Array.isArray";
    delete_is_out: "delete o.a" => "SyntaxError 1:1: delete is not in the subset: build the object without the key";
    void_is_out: "return void 0" => "SyntaxError 1:8: void is not in the subset: use undefined";
    comma_is_out: "a(), b()" => "SyntaxError 1:4: the comma operator is not in the subset: use one statement each";
    comma_in_parens_is_out: "const x = (a, b)" => "SyntaxError 1:13: the comma operator is not in the subset: use one statement each";
    bitwise_and_is_out: "const x = a & b" => "SyntaxError 1:13: the bitwise operator & is not in the subset";
    bitwise_or_is_out: "const x = a | 0" => "SyntaxError 1:13: the bitwise operator | is not in the subset";
    bitwise_xor_is_out: "const x = a ^ b" => "SyntaxError 1:13: the bitwise operator ^ is not in the subset";
    bitwise_not_is_out: "const x = ~a" => "SyntaxError 1:11: the bitwise operator ~ is not in the subset";
    shift_is_out: "const x = a << 2" => "SyntaxError 1:13: the bitwise operator << is not in the subset";
    unsigned_shift_is_out: "const x = a >>> 0" => "SyntaxError 1:13: the bitwise operator >>> is not in the subset";
    bitwise_assign_is_out: "a |= 1" => "SyntaxError 1:3: the bitwise operator |= is not in the subset";
    remainder_assign_is_out: "a %= 2" => "SyntaxError 1:3: the operator %= is not in the subset: write x = x % y";
    super_is_out: "const o = { f() { return super.f() } }" => "SyntaxError 1:26: super is not in the subset: there are no classes";
    debugger_is_out: "debugger" => "SyntaxError 1:1: debugger is not in the subset: use console.log";
    for_await_is_out: "for await (const x of xs) {}" => "SyntaxError 1:5: for await is not in the subset: use Promise.all";
    destructuring_assignment_is_out: "[a, b] = [b, a]" => "SyntaxError 1:8: destructuring assignment is not in the subset: declare with const [a, b] = … or const { a } = …";
    array_hole_is_out: "const a = [1, , 2]" => "SyntaxError 1:15: an array hole is not in the subset: write undefined";
    private_name_is_out: "o.#x" => "SyntaxError 1:3: '#' is not in the subset";
    await_outside_async_is_out: "const f = () => await g()" => "SyntaxError 1:17: await is only valid in an async function";
    break_outside_a_loop_is_out: "break" => "SyntaxError 1:1: break is only valid inside a loop";
    unary_left_of_pow_is_out: "const x = -2 ** 2" => "SyntaxError 1:14: put the left of ** in parentheses: (-a) ** b";
    deep_nesting_is_a_syntax_error: &"[".repeat(100_000) => "SyntaxError 1:201: nesting is deeper than 200 levels: split the code into functions";
}

#[test]
fn deep_prefix_chains_are_a_syntax_error() {
    for code in [
        "!".repeat(100_000),
        "a = ".repeat(100_000),
        "x => ".repeat(100_000),
        "(".repeat(100_000),
    ] {
        assert!(
            refused(&code).contains("nesting is deeper than 200 levels"),
            "{code:.10}"
        );
    }
}

#[test]
fn nesting_up_to_200_levels_parses() {
    let code = format!("const a = {}1{}", "[".repeat(199), "]".repeat(199));
    assert!(parse(&code).is_ok());
}

#[test]
fn columns_count_code_points() {
    assert_eq!(
        refused("const s = \"日本語\"; var x"),
        "SyntaxError 1:18: var is not in the subset: use let, or const"
    );
}

#[test]
fn the_worked_examples_parse() {
    let blocked = r#"
const lanes = await shed.ls({ labels: { fleet: "shed" } });
const blocked = lanes.filter((t) => t.reported?.state === "blocked");
return Promise.all(
  blocked.map(async (t) => {
    const { text } = await shed.read({ name: t.name, lines: 15 });
    return { name: t.name, kind: t.reported.kind ?? null, note: t.reported.note ?? null, tail: text.split("\n") };
  }),
);
"#;
    let start = r#"
const lane = await shed.run({
  name: "impl-192",
  cwd: "/home/user/work/shed/issue-192",
  cmd: ["claude"],
  labels: { fleet: "shed", issue: "192", role: "impl" },
  idle_after: 5,
});
await shed.wait({ name: lane.name, until: ["idle"], timeout: 20 });
await shed.send({ name: lane.name, text: "Read .brief.md fully, then start." });
const { outcome, terminal } = await shed.wait({ name: lane.name, until: ["done", "blocked", "error"], timeout: 30 });
return { name: lane.name, pid: lane.pid, outcome, state: terminal.reported?.state ?? null, note: terminal.reported?.note ?? null };
"#;
    let errors = r#"
const { text } = await shed.read({ name: "impl-190", history: true });
const lines = text.split("\n");
const kept = lines
  .map((line, i) => ({ n: i + 1, line: line.trim() }))
  .filter(({ line }) => line.matches("^error\\[E\\d+\\]") || line.startsWith("test result: FAILED"));
console.log(`${lines.length} lines read, ${kept.length} kept`);
return kept.slice(-20);
"#;
    for code in [blocked, start, errors] {
        let script = parse(code).unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(script.body.last(), Some(Stmt::Return(_, Some(_)))));
    }
}

#[test]
fn the_rest_of_the_subset_parses() {
    let code = r#"
let n = 0, m
for (let i = 0; i < 3; i++) { n += i; if (i === 1) continue; else break }
for (const [k, v = 1, ...more] of Object.entries({ a: 1 })) { m = k }
while (n > 0) n--
function f(a, { b, c: [d] = [] } = {}, ...rest) { return a ** 2 % 3 }
const g = async x => await x, h = async (a, b = `x${`y${1}`}z`) => { return a?.[b]?.(1) }
const o = { a, "b": 1, 2: 3, [k]: 4, ...p, m() {}, async q() {}, get: 1, set() {} }
try { throw new Error("no") } catch { m ??= 1 }
try { x } catch ({ message }) { m ||= message; m &&= typeof m }
const t = a == null ? -1 : !b != null ? +c : (d || e) && f
const u = 0x1F + 0b10 + 0o7 + 1_000 + .5e1 + 'it\'s \u{1F600}'
const yes = async function named() {}
return [...xs, u / 2 - 1 * 3]
"#;
    assert!(parse(code).is_ok(), "{:?}", parse(code).err());
}

#[test]
fn semicolons_are_optional_as_in_javascript() {
    let script = parse("const a = 1\nconst b = a\nreturn\na").expect("parses");
    assert_eq!(script.body.len(), 4);
    assert!(matches!(script.body.get(2), Some(Stmt::Return(_, None))));
    assert_eq!(
        refused("const a = 1 const b = 2"),
        "SyntaxError 1:13: unexpected const"
    );
}

#[test]
fn new_error_is_a_call_and_eq_null_a_null_check() {
    let script = parse("throw new Error(m)\nreturn x != null").expect("parses");
    let Some(Stmt::Throw(_, Expr::Call(_, callee, args, false))) = script.body.first() else {
        panic!("{script:?}");
    };
    assert!(matches!(&**callee, Expr::Name(_, name) if name == "Error"));
    assert!(matches!(args.as_slice(), [Item::Value(Expr::Name(_, m))] if m == "m"));
    assert!(matches!(
        script.body.get(1),
        Some(Stmt::Return(_, Some(Expr::NullCheck(_, true, _))))
    ));
}

#[test]
fn unclosed_things_say_where_they_start() {
    assert_eq!(
        refused("const s = \"abc"),
        "SyntaxError 1:11: a string is not closed"
    );
    assert_eq!(
        refused("const s = `abc${1}"),
        "SyntaxError 1:19: a template string is not closed"
    );
    assert_eq!(
        refused("/* never"),
        "SyntaxError 1:1: a /* comment is not closed"
    );
    assert_eq!(
        refused("if (a) {\n  b()\n"),
        "SyntaxError 1:8: a { is not closed"
    );
}
