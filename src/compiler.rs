//! Compiles the existing AST (same lexer/parser as the tree-walker) into
//! bytecode `Chunk`s. Variable references are resolved to stack slot
//! indices or upvalue indices *at compile time* instead of walking a
//! HashMap-based scope chain at every access — that resolution work is
//! the main reason a bytecode VM is faster than a tree-walker for
//! variable-heavy code.

use crate::ast::*;
use crate::chunk::{Chunk, OpCode};
use crate::value::{FunctionObj, Value};
use std::collections::HashSet;
use std::rc::Rc;

struct LocalVar {
    name: String,
    depth: usize,
    slot: u8,
    is_const: bool,
}

struct LoopCtx {
    break_jumps: Vec<usize>,
    continue_jumps: Vec<usize>,
}

struct FunctionState {
    chunk: Chunk,
    locals: Vec<LocalVar>,
    scope_depth: usize,
    next_slot: u16,
    upvalues: Vec<(bool, u8)>, // (is_local, index)
    loops: Vec<LoopCtx>,
    name: String,
    arity: usize,
}

impl FunctionState {
    fn new(name: &str, is_method: bool) -> Self {
        let mut fs = FunctionState {
            chunk: Chunk::new(),
            locals: Vec::new(),
            scope_depth: 0,
            next_slot: 1, // slot 0 is always reserved (self, or unused for plain functions)
            upvalues: Vec::new(),
            loops: Vec::new(),
            name: name.to_string(),
            arity: 0,
        };
        if is_method {
            fs.locals.push(LocalVar { name: "self".into(), depth: 0, slot: 0, is_const: false });
        }
        fs
    }
}

enum VarLoc { Local(u8), Upvalue(u8), Global }

pub struct Compiler {
    states: Vec<FunctionState>,
    /// Names declared with `const` at script (global) scope. Local `const`s
    /// are tracked per-function on `LocalVar` instead, since they naturally
    /// go out of scope with the rest of that block's locals.
    ///
    /// Scope note: this only sees `const` declarations made within the
    /// single source text being compiled right now. The REPL compiles each
    /// line as its own fresh `Compiler` against the same long-lived globals,
    /// so a `const` declared on one REPL line isn't remembered when
    /// compiling the next — reassigning it later in the same session won't
    /// be caught. Within one script file (or one REPL line), it's fully
    /// enforced.
    global_consts: HashSet<String>,
}

impl Compiler {
    pub fn compile_program(stmts: &[Stmt]) -> Result<Rc<FunctionObj>, String> {
        let mut c = Compiler { states: vec![FunctionState::new("<script>", false)], global_consts: HashSet::new() };
        c.compile_block(stmts)?;
        c.emit(OpCode::Nil);
        c.emit(OpCode::Return);
        let fs = c.states.pop().unwrap();
        let local_count = fs.next_slot as usize;
        Ok(Rc::new(FunctionObj { name: fs.name, arity: fs.arity, required_arity: fs.arity, local_count, chunk: fs.chunk, upvalue_count: fs.upvalues.len() }))
    }

    // ---- small helpers operating on the current (innermost) function ----
    fn cur(&mut self) -> &mut FunctionState { self.states.last_mut().unwrap() }
    fn emit(&mut self, op: OpCode) -> usize { self.cur().chunk.emit_op(op) }
    fn emit_u8(&mut self, b: u8) { self.cur().chunk.emit_u8(b); }
    fn emit_u16(&mut self, v: u16) { self.cur().chunk.emit_u16(v); }
    fn const_val(&mut self, v: Value) -> u16 { self.cur().chunk.add_constant(v) }
    fn const_str(&mut self, s: &str) -> u16 { self.const_val(Value::Str(Rc::new(s.to_string()))) }
    fn here(&mut self) -> usize { self.cur().chunk.code.len() }

    fn emit_jump(&mut self, op: OpCode) -> usize {
        self.emit(op);
        self.emit_u16(0xffff);
        self.here() - 2
    }
    fn patch_jump(&mut self, at: usize) {
        let target = self.here() as u16;
        self.cur().chunk.patch_u16(at, target);
    }
    fn emit_loop(&mut self, to: usize) {
        self.emit(OpCode::Loop);
        self.emit_u16(to as u16);
    }

    fn begin_scope(&mut self) { self.cur().scope_depth += 1; }
    fn end_scope(&mut self) {
        let depth = self.cur().scope_depth;
        self.cur().locals.retain(|l| l.depth < depth);
        self.cur().scope_depth -= 1;
    }

    fn declare_local(&mut self, name: &str, is_const: bool) -> Result<u8, String> {
        let fs = self.cur();
        if fs.next_slot > 250 { return Err(format!("too many local variables in '{}'", fs.name)); }
        let slot = fs.next_slot as u8;
        fs.next_slot += 1;
        let depth = fs.scope_depth;
        fs.locals.push(LocalVar { name: name.to_string(), depth, slot, is_const });
        Ok(slot)
    }

    fn find_local(fs: &FunctionState, name: &str) -> Option<u8> {
        fs.locals.iter().rev().find(|l| l.name == name).map(|l| l.slot)
    }

    fn add_upvalue(&mut self, level: usize, is_local: bool, index: u8) -> u8 {
        let fs = &mut self.states[level];
        if let Some(pos) = fs.upvalues.iter().position(|&(il, i)| il == is_local && i == index) {
            return pos as u8;
        }
        fs.upvalues.push((is_local, index));
        (fs.upvalues.len() - 1) as u8
    }

    /// Resolves `name` as visible at `level`, threading upvalue captures
    /// through every intervening function boundary as needed.
    fn resolve_at(&mut self, level: usize, name: &str) -> VarLoc {
        if let Some(slot) = Self::find_local(&self.states[level], name) {
            return VarLoc::Local(slot);
        }
        if level == 0 { return VarLoc::Global; }
        match self.resolve_at(level - 1, name) {
            VarLoc::Local(slot) => VarLoc::Upvalue(self.add_upvalue(level, true, slot)),
            VarLoc::Upvalue(idx) => VarLoc::Upvalue(self.add_upvalue(level, false, idx)),
            VarLoc::Global => VarLoc::Global,
        }
    }
    fn resolve(&mut self, name: &str) -> VarLoc {
        let level = self.states.len() - 1;
        self.resolve_at(level, name)
    }

    fn is_top_level(&self) -> bool { self.states.len() == 1 && self.cur_immut().scope_depth == 0 }
    fn cur_immut(&self) -> &FunctionState { self.states.last().unwrap() }

    /// Declares `name` for a `let`/`const`/`for`/`catch` binding, choosing
    /// global vs. local storage the same way the rest of the compiler does.
    fn declare_binding(&mut self, name: &str, is_const: bool) -> Result<(), String> {
        if self.is_top_level() {
            if is_const { self.global_consts.insert(name.to_string()); } else { self.global_consts.remove(name); }
            let idx = self.const_str(name);
            self.emit(OpCode::DefineGlobal);
            self.emit_u16(idx);
        } else {
            let slot = self.declare_local(name, is_const)?;
            self.emit(OpCode::SetLocal);
            self.emit_u8(slot);
            self.emit(OpCode::Pop);
        }
        Ok(())
    }

    /// Mirrors `resolve_at`'s local → upvalue-chain → global walk, but
    /// answers "is this binding const" instead of "where does it live".
    fn is_const_at(&self, level: usize, name: &str) -> bool {
        if let Some(l) = self.states[level].locals.iter().rev().find(|l| l.name == name) { return l.is_const; }
        if level == 0 { return self.global_consts.contains(name); }
        self.is_const_at(level - 1, name)
    }
    fn is_const_name(&self, name: &str) -> bool {
        let level = self.states.len() - 1;
        self.is_const_at(level, name)
    }

    fn get_var(&mut self, name: &str) {
        match self.resolve(name) {
            VarLoc::Local(slot) => { self.emit(OpCode::GetLocal); self.emit_u8(slot); }
            VarLoc::Upvalue(idx) => { self.emit(OpCode::GetUpvalue); self.emit_u8(idx); }
            VarLoc::Global => { let idx = self.const_str(name); self.emit(OpCode::GetGlobal); self.emit_u16(idx); }
        }
    }
    fn set_var(&mut self, name: &str) {
        match self.resolve(name) {
            VarLoc::Local(slot) => { self.emit(OpCode::SetLocal); self.emit_u8(slot); }
            VarLoc::Upvalue(idx) => { self.emit(OpCode::SetUpvalue); self.emit_u8(idx); }
            VarLoc::Global => { let idx = self.const_str(name); self.emit(OpCode::SetGlobal); self.emit_u16(idx); }
        }
    }

    // ---- statements ----
    fn compile_block(&mut self, stmts: &[Stmt]) -> Result<(), String> {
        for s in stmts { self.compile_stmt(s)?; }
        Ok(())
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> Result<(), String> {
        match stmt {
            Stmt::ExprStmt(e) => { self.compile_expr(e)?; self.emit(OpCode::Pop); }
            Stmt::Let { name, value } => {
                self.compile_expr(value)?;
                self.declare_binding(name, false)?;
            }
            Stmt::Const { name, value } => {
                self.compile_expr(value)?;
                self.declare_binding(name, true)?;
            }
            Stmt::LetDestructure { names, value, is_const } => {
                self.compile_expr(value)?;
                // No begin_scope/end_scope here on purpose: that would flip
                // is_top_level() to false partway through and silently turn
                // a top-level `let (a, b) = f();` into locals instead of
                // globals, unlike plain `let`. The scratch slot below just
                // permanently occupies one extra local slot in this
                // function — harmless, and its name (with a leading space)
                // can never collide with anything the user can write.
                let scratch = self.declare_local(" destructure", false)?;
                self.emit(OpCode::SetLocal); self.emit_u8(scratch); self.emit(OpCode::Pop);
                for (i, name) in names.iter().enumerate() {
                    self.emit(OpCode::GetLocal); self.emit_u8(scratch);
                    let idx_const = self.const_val(Value::Int(i as i64));
                    self.emit(OpCode::Constant); self.emit_u16(idx_const);
                    self.emit(OpCode::GetIndex);
                    self.declare_binding(name, *is_const)?;
                }
            }
            Stmt::If { branches, else_branch } => {
                let mut end_jumps = Vec::new();
                for (cond, body) in branches {
                    self.compile_expr(cond)?;
                    let skip = self.emit_jump(OpCode::JumpIfFalse);
                    self.emit(OpCode::Pop);
                    self.begin_scope();
                    self.compile_block(body)?;
                    self.end_scope();
                    end_jumps.push(self.emit_jump(OpCode::Jump));
                    self.patch_jump(skip);
                    self.emit(OpCode::Pop);
                }
                if let Some(body) = else_branch {
                    self.begin_scope();
                    self.compile_block(body)?;
                    self.end_scope();
                }
                for j in end_jumps { self.patch_jump(j); }
            }
            Stmt::While { cond, body } => {
                let loop_start = self.here();
                self.compile_expr(cond)?;
                let exit = self.emit_jump(OpCode::JumpIfFalse);
                self.emit(OpCode::Pop);
                self.cur().loops.push(LoopCtx { break_jumps: Vec::new(), continue_jumps: Vec::new() });
                self.begin_scope();
                self.compile_block(body)?;
                self.end_scope();
                let ctx = self.cur().loops.pop().unwrap();
                for j in ctx.continue_jumps { self.patch_jump(j); }
                self.emit_loop(loop_start);
                self.patch_jump(exit);
                self.emit(OpCode::Pop);
                for j in ctx.break_jumps { self.patch_jump(j); }
            }
            Stmt::For { var, iter, body } => {
                self.compile_expr(iter)?;
                self.emit(OpCode::GetIterList);
                self.begin_scope();
                let list_slot = self.declare_local(" for_list", false)?;
                self.emit(OpCode::SetLocal); self.emit_u8(list_slot); self.emit(OpCode::Pop);
                self.emit(OpCode::Constant);
                let zero = self.const_val(Value::Int(0));
                self.emit_u16(zero);
                let idx_slot = self.declare_local(" for_idx", false)?;
                self.emit(OpCode::SetLocal); self.emit_u8(idx_slot); self.emit(OpCode::Pop);

                let loop_start = self.here();
                self.emit(OpCode::GetLocal); self.emit_u8(idx_slot);
                self.emit(OpCode::GetLocal); self.emit_u8(list_slot);
                self.emit(OpCode::Len);
                self.emit(OpCode::Less);
                let exit = self.emit_jump(OpCode::JumpIfFalse);
                self.emit(OpCode::Pop);

                self.emit(OpCode::GetLocal); self.emit_u8(list_slot);
                self.emit(OpCode::GetLocal); self.emit_u8(idx_slot);
                self.emit(OpCode::GetIndex);
                self.begin_scope();
                let var_slot = self.declare_local(var, false)?;
                self.emit(OpCode::SetLocal); self.emit_u8(var_slot); self.emit(OpCode::Pop);

                self.cur().loops.push(LoopCtx { break_jumps: Vec::new(), continue_jumps: Vec::new() });
                self.compile_block(body)?;
                let ctx = self.cur().loops.pop().unwrap();
                for j in ctx.continue_jumps { self.patch_jump(j); }
                self.end_scope();

                self.emit(OpCode::GetLocal); self.emit_u8(idx_slot);
                self.emit(OpCode::Constant);
                let one = self.const_val(Value::Int(1));
                self.emit_u16(one);
                self.emit(OpCode::Add);
                self.emit(OpCode::SetLocal); self.emit_u8(idx_slot); self.emit(OpCode::Pop);
                self.emit_loop(loop_start);

                self.patch_jump(exit);
                self.emit(OpCode::Pop);
                for j in ctx.break_jumps { self.patch_jump(j); }
                self.end_scope();
            }
            Stmt::FuncDecl { name, params, body } => {
                self.compile_function(name, params, body, false)?;
                self.declare_binding(name, false)?;
            }
            Stmt::ClassDecl { name, parent, methods } => {
                if let Some(p) = parent { self.get_var(p); } else { self.emit(OpCode::Nil); }
                let name_idx = self.const_str(name);
                self.emit(OpCode::Class);
                self.emit_u16(name_idx);
                self.declare_binding(name, false)?;
                self.get_var(name);
                for m in methods {
                    if let Stmt::FuncDecl { name: mname, params, body } = m {
                        self.compile_function(mname, params, body, true)?;
                        let midx = self.const_str(mname);
                        self.emit(OpCode::Method);
                        self.emit_u16(midx);
                    }
                }
                self.emit(OpCode::Pop);
            }
            Stmt::Return(e) => {
                match e { Some(e) => self.compile_expr(e)?, None => { self.emit(OpCode::Nil); } }
                self.emit(OpCode::Return);
            }
            Stmt::Break => {
                let j = self.emit_jump(OpCode::Jump);
                match self.cur().loops.last_mut() {
                    Some(l) => l.break_jumps.push(j),
                    None => return Err("'break' used outside of a loop".into()),
                }
            }
            Stmt::Continue => {
                let j = self.emit_jump(OpCode::Jump);
                match self.cur().loops.last_mut() {
                    Some(l) => l.continue_jumps.push(j),
                    None => return Err("'continue' used outside of a loop".into()),
                }
            }
            Stmt::Throw(e) => { self.compile_expr(e)?; self.emit(OpCode::Throw); }
            Stmt::TryCatch { try_block, err_name, catch_block } => {
                let handler = self.emit_jump(OpCode::PushTry);
                self.begin_scope();
                self.compile_block(try_block)?;
                self.end_scope();
                self.emit(OpCode::PopTry);
                let skip_catch = self.emit_jump(OpCode::Jump);
                self.patch_jump(handler);
                self.begin_scope();
                let slot = self.declare_local(err_name, false)?;
                self.emit(OpCode::SetLocal); self.emit_u8(slot); self.emit(OpCode::Pop);
                self.compile_block(catch_block)?;
                self.end_scope();
                self.patch_jump(skip_catch);
            }
            Stmt::Import(path) => {
                let code = std::fs::read_to_string(path).map_err(|e| format!("cannot import '{}': {}", path, e))?;
                let tokens = crate::lexer::tokenize(&code)?;
                let stmts = crate::parser::parse(tokens)?;
                self.compile_block(&stmts)?;
            }
            Stmt::ImportAs(path, alias) => {
                let code = std::fs::read_to_string(path).map_err(|e| format!("cannot import '{}': {}", path, e))?;
                let tokens = crate::lexer::tokenize(&code)?;
                let stmts = crate::parser::parse(tokens)?;
                // Same textual inlining as a plain import, but we also note
                // every top-level name the file declares so we can gather
                // them into a dict afterward — that's what makes
                // `alias.someFn()` / `alias.someValue` work.
                let mut export_names = Vec::new();
                for s in &stmts {
                    match s {
                        Stmt::Let { name, .. } | Stmt::Const { name, .. } => export_names.push(name.clone()),
                        Stmt::FuncDecl { name, .. } => export_names.push(name.clone()),
                        Stmt::ClassDecl { name, .. } => export_names.push(name.clone()),
                        Stmt::LetDestructure { names, .. } => export_names.extend(names.iter().cloned()),
                        _ => {}
                    }
                }
                self.compile_block(&stmts)?;
                for name in &export_names {
                    let key_idx = self.const_str(name);
                    self.emit(OpCode::Constant); self.emit_u16(key_idx);
                    self.get_var(name);
                }
                self.emit(OpCode::BuildDict);
                self.emit_u16(export_names.len() as u16);
                self.declare_binding(alias, false)?;
            }
        }
        Ok(())
    }

    fn compile_function(&mut self, name: &str, params: &[Param], body: &[Stmt], is_method: bool) -> Result<(), String> {
        self.states.push(FunctionState::new(name, is_method));
        self.cur().arity = params.len();
        let required_arity = params.iter().filter(|p| p.default.is_none()).count();
        for p in params {
            let slot = self.declare_local(&p.name, false)?;
            if let Some(default) = &p.default {
                self.emit(OpCode::GetLocal); self.emit_u8(slot);
                self.emit(OpCode::Nil);
                self.emit(OpCode::Equal);
                let skip = self.emit_jump(OpCode::JumpIfFalse);
                self.emit(OpCode::Pop);
                self.compile_expr(default)?;
                self.emit(OpCode::SetLocal); self.emit_u8(slot);
                self.emit(OpCode::Pop);
                let after = self.emit_jump(OpCode::Jump);
                self.patch_jump(skip);
                self.emit(OpCode::Pop);
                self.patch_jump(after);
            }
        }
        self.compile_block(body)?;
        self.emit(OpCode::Nil);
        self.emit(OpCode::Return);

        let fs = self.states.pop().unwrap();
        let upvalues = fs.upvalues.clone();
        let local_count = fs.next_slot as usize;
        let func = Rc::new(FunctionObj { name: fs.name, arity: fs.arity, required_arity, local_count, chunk: fs.chunk, upvalue_count: fs.upvalues.len() });
        // The constant holds a *template* closure (no upvalues attached yet); the
        // Closure opcode below builds the real runtime closure with actual
        // captured upvalues from it when this code executes.
        let template = Value::Closure(Rc::new(crate::value::ClosureObj { function: func, upvalues: Vec::new() }));
        let fn_idx = self.const_val(template);
        self.emit(OpCode::Closure);
        self.emit_u16(fn_idx);
        for (is_local, index) in upvalues {
            self.emit_u8(is_local as u8);
            self.emit_u8(index);
        }
        Ok(())
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<(), String> {
        match expr {
            Expr::Int(i) => { let idx = self.const_val(Value::Int(*i)); self.emit(OpCode::Constant); self.emit_u16(idx); }
            Expr::Float(f) => { let idx = self.const_val(Value::Float(*f)); self.emit(OpCode::Constant); self.emit_u16(idx); }
            Expr::Str(s) => { let idx = self.const_str(s); self.emit(OpCode::Constant); self.emit_u16(idx); }
            Expr::Bool(true) => { self.emit(OpCode::True); }
            Expr::Bool(false) => { self.emit(OpCode::False); }
            Expr::Null => { self.emit(OpCode::Nil); }
            Expr::Interp(parts) => {
                if parts.is_empty() {
                    let idx = self.const_str("");
                    self.emit(OpCode::Constant); self.emit_u16(idx);
                } else {
                    for (i, p) in parts.iter().enumerate() {
                        match p {
                            InterpPart::Lit(s) => { let idx = self.const_str(s); self.emit(OpCode::Constant); self.emit_u16(idx); }
                            InterpPart::Expr(e) => { self.compile_expr(e)?; self.emit(OpCode::ToStr); }
                        }
                        if i > 0 { self.emit(OpCode::Add); }
                    }
                }
            }
            Expr::List(items) => {
                for it in items { self.compile_expr(it)?; }
                self.emit(OpCode::BuildList);
                self.emit_u16(items.len() as u16);
            }
            Expr::Dict(pairs) => {
                for (k, v) in pairs { self.compile_expr(k)?; self.compile_expr(v)?; }
                self.emit(OpCode::BuildDict);
                self.emit_u16(pairs.len() as u16);
            }
            Expr::Ident(name) => self.get_var(name),
            Expr::Unary { op, expr } => {
                self.compile_expr(expr)?;
                match op.as_str() { "-" => { self.emit(OpCode::Negate); } "!" => { self.emit(OpCode::Not); } _ => unreachable!() }
            }
            Expr::Binary { op, left, right } => {
                self.compile_expr(left)?;
                self.compile_expr(right)?;
                self.emit(match op.as_str() {
                    "+" => OpCode::Add, "-" => OpCode::Subtract, "*" => OpCode::Multiply, "/" => OpCode::Divide,
                    "%" => OpCode::Modulo, "**" => OpCode::Power, "@" => OpCode::MatMul,
                    "==" => OpCode::EqUser, "!=" => OpCode::NotEqUser, "<" => OpCode::Less, ">" => OpCode::Greater,
                    "<=" => OpCode::LessEqual, ">=" => OpCode::GreaterEqual,
                    other => return Err(format!("unknown operator '{}'", other)),
                });
            }
            Expr::Logical { op, left, right } => {
                self.compile_expr(left)?;
                if op == "and" {
                    let end = self.emit_jump(OpCode::JumpIfFalse);
                    self.emit(OpCode::Pop);
                    self.compile_expr(right)?;
                    self.patch_jump(end);
                } else {
                    let else_j = self.emit_jump(OpCode::JumpIfFalse);
                    let end = self.emit_jump(OpCode::Jump);
                    self.patch_jump(else_j);
                    self.emit(OpCode::Pop);
                    self.compile_expr(right)?;
                    self.patch_jump(end);
                }
            }
            Expr::Assign { name, value } => {
                if self.is_const_name(name) { return Err(format!("cannot assign to '{}': it was declared with 'const'", name)); }
                self.compile_expr(value)?;
                self.set_var(name);
            }
            Expr::IndexAssign { target, index, value } => {
                self.compile_expr(target)?;
                self.compile_expr(index)?;
                self.compile_expr(value)?;
                self.emit(OpCode::SetIndex);
            }
            Expr::FieldAssign { target, field, value } => {
                self.compile_expr(target)?;
                self.compile_expr(value)?;
                let idx = self.const_str(field);
                self.emit(OpCode::SetProperty);
                self.emit_u16(idx);
            }
            Expr::Index { target, index } => {
                self.compile_expr(target)?;
                self.compile_expr(index)?;
                self.emit(OpCode::GetIndex);
            }
            Expr::Field { target, name } => {
                self.compile_expr(target)?;
                let idx = self.const_str(name);
                self.emit(OpCode::GetProperty);
                self.emit_u16(idx);
            }
            Expr::New { class, args } => {
                self.get_var(class);
                for a in args { self.compile_expr(a)?; }
                self.emit(OpCode::Instantiate);
                self.emit_u8(args.len() as u8);
            }
            Expr::FuncExpr { params, body } => { self.compile_function("<anonymous>", params, body, false)?; }
            Expr::Call { callee, args } => {
                if let Expr::Field { target, name } = callee.as_ref() {
                    self.compile_expr(target)?;
                    for a in args { self.compile_expr(a)?; }
                    let idx = self.const_str(name);
                    self.emit(OpCode::Invoke);
                    self.emit_u16(idx);
                    self.emit_u8(args.len() as u8);
                } else {
                    self.compile_expr(callee)?;
                    for a in args { self.compile_expr(a)?; }
                    self.emit(OpCode::Call);
                    self.emit_u8(args.len() as u8);
                }
            }
            Expr::Ternary { cond, then_branch, else_branch } => {
                // Same bytecode shape as an `if` statement's branches, just
                // producing one value on the stack instead of side effects.
                self.compile_expr(cond)?;
                let else_jump = self.emit_jump(OpCode::JumpIfFalse);
                self.emit(OpCode::Pop);
                self.compile_expr(then_branch)?;
                let end_jump = self.emit_jump(OpCode::Jump);
                self.patch_jump(else_jump);
                self.emit(OpCode::Pop);
                self.compile_expr(else_branch)?;
                self.patch_jump(end_jump);
            }
            Expr::Match { subject, arms, default } => {
                self.compile_expr(subject)?;
                let scratch = self.declare_local(" match", false)?;
                self.emit(OpCode::SetLocal); self.emit_u8(scratch); self.emit(OpCode::Pop);
                let mut end_jumps = Vec::new();
                for (pattern, body) in arms {
                    self.emit(OpCode::GetLocal); self.emit_u8(scratch);
                    self.compile_expr(pattern)?;
                    self.emit(OpCode::EqUser);
                    let next_arm = self.emit_jump(OpCode::JumpIfFalse);
                    self.emit(OpCode::Pop);
                    self.compile_expr(body)?;
                    end_jumps.push(self.emit_jump(OpCode::Jump));
                    self.patch_jump(next_arm);
                    self.emit(OpCode::Pop);
                }
                match default {
                    Some(body) => self.compile_expr(body)?,
                    None => {
                        let msg = self.const_str("no match arm matched, and there was no '_' default");
                        self.emit(OpCode::Constant); self.emit_u16(msg);
                        self.emit(OpCode::Throw);
                    }
                }
                for j in end_jumps { self.patch_jump(j); }
            }
        }
        Ok(())
    }
}
