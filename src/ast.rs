#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub default: Option<Expr>,
}

#[derive(Debug, Clone)]
pub enum InterpPart {
    Lit(String),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Null,
    Interp(Vec<InterpPart>),
    List(Vec<Expr>),
    Dict(Vec<(Expr, Expr)>),
    Ident(String),
    Unary { op: String, expr: Box<Expr> },
    Binary { op: String, left: Box<Expr>, right: Box<Expr> },
    Logical { op: String, left: Box<Expr>, right: Box<Expr> },
    Assign { name: String, value: Box<Expr> },
    IndexAssign { target: Box<Expr>, index: Box<Expr>, value: Box<Expr> },
    FieldAssign { target: Box<Expr>, field: String, value: Box<Expr> },
    Call { callee: Box<Expr>, args: Vec<Expr> },
    Index { target: Box<Expr>, index: Box<Expr> },
    Field { target: Box<Expr>, name: String },
    New { class: String, args: Vec<Expr> },
    FuncExpr { params: Vec<Param>, body: Vec<Stmt> },
    Ternary { cond: Box<Expr>, then_branch: Box<Expr>, else_branch: Box<Expr> },
    Match { subject: Box<Expr>, arms: Vec<(Expr, Expr)>, default: Option<Box<Expr>> },
}

#[derive(Debug, Clone)]
pub enum Stmt {
    ExprStmt(Expr),
    Let { name: String, value: Expr },
    Const { name: String, value: Expr },
    /// `let (a, b) = expr;` — unpacks a list by position. `is_const` mirrors
    /// `Const` above: it makes every name in the pattern immutable, not the
    /// unpacked values themselves.
    LetDestructure { names: Vec<String>, value: Expr, is_const: bool },
    If { branches: Vec<(Expr, Vec<Stmt>)>, else_branch: Option<Vec<Stmt>> },
    While { cond: Expr, body: Vec<Stmt> },
    For { var: String, iter: Expr, body: Vec<Stmt> },
    FuncDecl { name: String, params: Vec<Param>, body: Vec<Stmt> },
    ClassDecl { name: String, parent: Option<String>, methods: Vec<Stmt> },
    Return(Option<Expr>),
    Break,
    Continue,
    TryCatch { try_block: Vec<Stmt>, err_name: String, catch_block: Vec<Stmt> },
    Throw(Expr),
    Import(String),
    /// `import "file.k" as name;` — same textual inlining as `Import`, but
    /// also collects the file's top-level names into a dict bound to `name`,
    /// so `name.someFn()` / `name.someValue` works like a namespace.
    ImportAs(String, String),
}
