//! The tree a program parses to: only what the subset has, so a left-out construct cannot be built.

/// Where a node starts: the line and the column, both from 1, the column in code points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pos {
    /// The line, from 1.
    pub line: u32,
    /// The column, from 1.
    pub column: u32,
}

/// A whole program: the body of an async function.
#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    /// Its statements, in order.
    pub body: Vec<Stmt>,
}

/// `const` or `let`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decl {
    /// `const`.
    Const,
    /// `let`.
    Let,
}

/// A binding target with its default: `x = 1`, `{ a } = {}`.
pub type Binding = (Pattern, Option<Expr>);

/// What a `const`, `let`, parameter or `catch` binds.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// One name.
    Name(Pos, String),
    /// `{ a, b: c = 1, [k]: d, ...rest }`.
    Object(Vec<(Key, Binding)>, Option<Box<Pattern>>),
    /// `[a, , b = 1, ...rest]`; `None` is a hole.
    Array(Vec<Option<Binding>>, Option<Box<Pattern>>),
}

/// A property's key in an object literal or pattern.
#[derive(Debug, Clone, PartialEq)]
pub enum Key {
    /// A name, string or number key, as its text.
    Name(String),
    /// `[expr]`.
    Computed(Box<Expr>),
}

/// A function, arrow or not.
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    /// Where it starts.
    pub pos: Pos,
    /// Its name, when it has one.
    pub name: Option<String>,
    /// `async`.
    pub is_async: bool,
    /// An arrow function.
    pub arrow: bool,
    /// Its parameters with their defaults.
    pub params: Vec<Binding>,
    /// Its `...rest` parameter.
    pub rest: Option<Pattern>,
    /// Its body; an arrow's expression body is one `return`.
    pub body: Vec<Stmt>,
}

/// A statement.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `const` or `let` with its bindings.
    Decl(Decl, Vec<Binding>),
    /// `function f() {}`.
    Function(Box<Function>),
    /// An expression on its own.
    Expr(Expr),
    /// `{ … }`.
    Block(Vec<Stmt>),
    /// `if (c) a else b`.
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    /// `for (const x of xs) body`.
    ForOf(Decl, Pattern, Expr, Box<Stmt>),
    /// `for (init; test; update) body`.
    For(Option<Box<Stmt>>, Option<Expr>, Option<Expr>, Box<Stmt>),
    /// `while (c) body`.
    While(Expr, Box<Stmt>),
    /// `break`.
    Break(Pos),
    /// `continue`.
    Continue(Pos),
    /// `return` with its value.
    Return(Pos, Option<Expr>),
    /// `throw value`.
    Throw(Pos, Expr),
    /// `try { … } catch (e) { … }`; the binding is optional.
    Try(Vec<Stmt>, Option<Pattern>, Vec<Stmt>),
    /// `;`.
    Empty,
}

/// An array element or call argument.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// A value.
    Value(Expr),
    /// `...value`.
    Spread(Expr),
}

/// An object literal's entry.
#[derive(Debug, Clone, PartialEq)]
pub enum Prop {
    /// `key: value`, a shorthand or a method.
    Value(Key, Expr),
    /// `...value`.
    Spread(Expr),
}

/// A prefix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unary {
    /// `!`.
    Not,
    /// `-`.
    Neg,
    /// `+`.
    Plus,
    /// `typeof`.
    Typeof,
}

/// A binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binary {
    /// `+`.
    Add,
    /// `-`.
    Sub,
    /// `*`.
    Mul,
    /// `/`.
    Div,
    /// `%`.
    Rem,
    /// `**`.
    Pow,
    /// `===`.
    Eq,
    /// `!==`.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
}

/// A short-circuit operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Logic {
    /// `&&`.
    And,
    /// `||`.
    Or,
    /// `??`.
    Nullish,
}

/// An assignment operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assign {
    /// `=`.
    Set,
    /// `+=`.
    Add,
    /// `-=`.
    Sub,
    /// `*=`.
    Mul,
    /// `/=`.
    Div,
    /// `&&=`.
    And,
    /// `||=`.
    Or,
    /// `??=`.
    Nullish,
}

/// An expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A number.
    Num(f64),
    /// A string.
    Str(String),
    /// A template string: its texts, one more than its values.
    Template(Vec<String>, Vec<Expr>),
    /// `true` or `false`.
    Bool(bool),
    /// `null`.
    Null,
    /// A name.
    Name(Pos, String),
    /// `[a, ...b]`.
    Array(Vec<Item>),
    /// `{ a, b: 1, [k]: v, ...o }`.
    Object(Vec<Prop>),
    /// A function or arrow.
    Function(Box<Function>),
    /// `!x`, `-x`, `+x`, `typeof x`.
    Unary(Pos, Unary, Box<Expr>),
    /// `++x`, `x--`: prefix, increment, target.
    Update(Pos, bool, bool, Box<Expr>),
    /// `a + b` and the other arithmetic and comparisons.
    Binary(Pos, Binary, Box<Expr>, Box<Expr>),
    /// `x == null` (`false`) or `x != null` (`true`).
    NullCheck(Pos, bool, Box<Expr>),
    /// `a && b`, `a || b`, `a ?? b`.
    Logic(Pos, Logic, Box<Expr>, Box<Expr>),
    /// `c ? a : b`.
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `target = value` and the compound forms.
    Assign(Pos, Assign, Box<Expr>, Box<Expr>),
    /// `object.name`, or `object?.name` when optional.
    Member(Pos, Box<Expr>, String, bool),
    /// `object[key]`, or `object?.[key]` when optional.
    Index(Pos, Box<Expr>, Box<Expr>, bool),
    /// `callee(args)`, or `callee?.(args)` when optional; `new Error(m)` is `Error(m)`.
    Call(Pos, Box<Expr>, Vec<Item>, bool),
    /// `await value`.
    Await(Pos, Box<Expr>),
}
