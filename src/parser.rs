//! Recursive-descent parser with precedence climbing for expressions.
//! Every error returns a `Result` with a line-free but descriptive message —
//! never panics, so a syntax error in a user script can be shown, not crash the process.

use crate::ast::*;
use crate::lexer::{FStringPart, Token};

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self { Parser { tokens, pos: 0 } }

    fn peek(&self) -> &Token { &self.tokens[self.pos] }
    fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos < self.tokens.len() - 1 { self.pos += 1; }
        t
    }
    fn check(&self, t: &Token) -> bool { self.peek() == t }
    fn match_tok(&mut self, t: &Token) -> bool { if self.check(t) { self.advance(); true } else { false } }
    fn expect(&mut self, t: Token, msg: &str) -> Result<(), String> {
        if self.check(&t) { self.advance(); Ok(()) } else { Err(format!("{}: got {:?}", msg, self.peek())) }
    }
    fn ident(&mut self) -> Result<String, String> {
        match self.advance() {
            Token::Ident(s) => Ok(s),
            other => Err(format!("expected identifier, got {:?}", other)),
        }
    }

    pub fn parse_program(&mut self) -> Result<Vec<Stmt>, String> {
        let mut stmts = Vec::new();
        while !self.check(&Token::Eof) { stmts.push(self.statement()?); }
        Ok(stmts)
    }

    fn block(&mut self) -> Result<Vec<Stmt>, String> {
        self.expect(Token::LBrace, "expected '{'")?;
        let mut stmts = Vec::new();
        while !self.check(&Token::RBrace) && !self.check(&Token::Eof) { stmts.push(self.statement()?); }
        self.expect(Token::RBrace, "expected '}'")?;
        Ok(stmts)
    }

    fn statement(&mut self) -> Result<Stmt, String> {
        match self.peek().clone() {
            Token::Let => { self.advance(); self.let_stmt(false) }
            Token::Const => { self.advance(); self.let_stmt(true) }
            Token::Fn => { self.advance(); self.func_decl() }
            Token::Class => { self.advance(); self.class_decl() }
            Token::If => { self.advance(); self.if_stmt() }
            Token::While => { self.advance(); self.while_stmt() }
            Token::For => { self.advance(); self.for_stmt() }
            Token::Import => {
                self.advance();
                let path = match self.advance() { Token::Str(s) => s, other => return Err(format!("expected string path after 'import', got {:?}", other)) };
                if self.match_tok(&Token::As) {
                    let alias = self.ident()?;
                    self.match_tok(&Token::Semi);
                    Ok(Stmt::ImportAs(path, alias))
                } else {
                    self.match_tok(&Token::Semi);
                    Ok(Stmt::Import(path))
                }
            }
            Token::Return => {
                self.advance();
                if self.check(&Token::Semi) || self.check(&Token::RBrace) { self.match_tok(&Token::Semi); Ok(Stmt::Return(None)) }
                else {
                    let mut exprs = vec![self.expression()?];
                    while self.match_tok(&Token::Comma) { exprs.push(self.expression()?); }
                    self.match_tok(&Token::Semi);
                    if exprs.len() == 1 { Ok(Stmt::Return(Some(exprs.into_iter().next().unwrap()))) }
                    // `return a, b;` packs into a list — `let (a, b) = f();` unpacks it back out.
                    else { Ok(Stmt::Return(Some(Expr::List(exprs)))) }
                }
            }
            Token::Break => { self.advance(); self.match_tok(&Token::Semi); Ok(Stmt::Break) }
            Token::Continue => { self.advance(); self.match_tok(&Token::Semi); Ok(Stmt::Continue) }
            Token::Try => { self.advance(); self.try_stmt() }
            Token::Throw => { self.advance(); let e = self.expression()?; self.match_tok(&Token::Semi); Ok(Stmt::Throw(e)) }
            _ => { let e = self.expression()?; self.match_tok(&Token::Semi); Ok(Stmt::ExprStmt(e)) }
        }
    }

    fn let_stmt(&mut self, is_const: bool) -> Result<Stmt, String> {
        if self.check(&Token::LParen) {
            self.advance();
            let mut names = Vec::new();
            loop {
                names.push(self.ident()?);
                if !self.match_tok(&Token::Comma) { break; }
            }
            self.expect(Token::RParen, "expected ')' after destructuring names")?;
            self.expect(Token::Assign, "expected '=' in destructuring binding")?;
            let value = self.expression()?;
            self.match_tok(&Token::Semi);
            return Ok(Stmt::LetDestructure { names, value, is_const });
        }
        let name = self.ident()?;
        if self.match_tok(&Token::Colon) { self.ident().ok(); } // optional type annotation (documentation only)
        self.expect(Token::Assign, "expected '=' in binding")?;
        let value = self.expression()?;
        self.match_tok(&Token::Semi);
        if is_const { Ok(Stmt::Const { name, value }) } else { Ok(Stmt::Let { name, value }) }
    }

    fn params(&mut self) -> Result<Vec<Param>, String> {
        self.expect(Token::LParen, "expected '('")?;
        let mut params = Vec::new();
        if !self.check(&Token::RParen) {
            loop {
                let name = self.ident()?;
                if self.match_tok(&Token::Colon) { self.ident().ok(); }
                let default = if self.match_tok(&Token::Assign) { Some(self.expression()?) } else { None };
                params.push(Param { name, default });
                if !self.match_tok(&Token::Comma) { break; }
            }
        }
        self.expect(Token::RParen, "expected ')'")?;
        Ok(params)
    }

    fn func_decl(&mut self) -> Result<Stmt, String> {
        let name = self.ident()?;
        let params = self.params()?;
        if self.match_tok(&Token::Arrow) { self.ident().ok(); } // return type annotation, documentation only
        let body = self.block()?;
        Ok(Stmt::FuncDecl { name, params, body })
    }

    fn class_decl(&mut self) -> Result<Stmt, String> {
        let name = self.ident()?;
        let parent = if self.match_tok(&Token::LParen) {
            let p = self.ident()?;
            self.expect(Token::RParen, "expected ')' after base class name")?;
            Some(p)
        } else { None };
        self.expect(Token::LBrace, "expected '{' to start class body")?;
        let mut methods = Vec::new();
        while !self.check(&Token::RBrace) && !self.check(&Token::Eof) {
            self.expect(Token::Fn, "class bodies may only contain methods (fn ...)")?;
            methods.push(self.func_decl()?);
        }
        self.expect(Token::RBrace, "expected '}'")?;
        Ok(Stmt::ClassDecl { name, parent, methods })
    }

    fn if_stmt(&mut self) -> Result<Stmt, String> {
        let mut branches = Vec::new();
        let cond = self.expression()?;
        let body = self.block()?;
        branches.push((cond, body));
        let mut else_branch = None;
        loop {
            if self.match_tok(&Token::Elif) {
                let c = self.expression()?;
                let b = self.block()?;
                branches.push((c, b));
            } else if self.match_tok(&Token::Else) {
                if self.check(&Token::If) {
                    self.advance();
                    let c = self.expression()?;
                    let b = self.block()?;
                    branches.push((c, b));
                } else {
                    else_branch = Some(self.block()?);
                    break;
                }
            } else { break; }
        }
        Ok(Stmt::If { branches, else_branch })
    }

    fn while_stmt(&mut self) -> Result<Stmt, String> {
        let cond = self.expression()?;
        let body = self.block()?;
        Ok(Stmt::While { cond, body })
    }

    fn for_stmt(&mut self) -> Result<Stmt, String> {
        let var = self.ident()?;
        self.expect(Token::In, "expected 'in' in for loop")?;
        let iter = self.expression()?;
        let body = self.block()?;
        Ok(Stmt::For { var, iter, body })
    }

    fn try_stmt(&mut self) -> Result<Stmt, String> {
        let try_block = self.block()?;
        self.expect(Token::Catch, "expected 'catch' after try block")?;
        let err_name = if let Token::Ident(_) = self.peek() { self.ident()? } else { "err".to_string() };
        let catch_block = self.block()?;
        Ok(Stmt::TryCatch { try_block, err_name, catch_block })
    }

    // ---- expressions, lowest to highest precedence ----
    fn expression(&mut self) -> Result<Expr, String> { self.assignment() }

    fn assignment(&mut self) -> Result<Expr, String> {
        let expr = self.ternary()?;
        if matches!(self.peek(), Token::Assign | Token::PlusAssign | Token::MinusAssign | Token::StarAssign | Token::SlashAssign | Token::PercentAssign | Token::StarStarAssign) {
            let op_tok = self.advance();
            let raw_value = self.assignment()?;
            let value = match op_tok {
                Token::PlusAssign => Expr::Binary { op: "+".into(), left: Box::new(expr.clone()), right: Box::new(raw_value) },
                Token::MinusAssign => Expr::Binary { op: "-".into(), left: Box::new(expr.clone()), right: Box::new(raw_value) },
                Token::StarAssign => Expr::Binary { op: "*".into(), left: Box::new(expr.clone()), right: Box::new(raw_value) },
                Token::SlashAssign => Expr::Binary { op: "/".into(), left: Box::new(expr.clone()), right: Box::new(raw_value) },
                Token::PercentAssign => Expr::Binary { op: "%".into(), left: Box::new(expr.clone()), right: Box::new(raw_value) },
                Token::StarStarAssign => Expr::Binary { op: "**".into(), left: Box::new(expr.clone()), right: Box::new(raw_value) },
                _ => raw_value,
            };
            return match expr {
                Expr::Ident(name) => Ok(Expr::Assign { name, value: Box::new(value) }),
                Expr::Index { target, index } => Ok(Expr::IndexAssign { target, index, value: Box::new(value) }),
                Expr::Field { target, name } => Ok(Expr::FieldAssign { target, field: name, value: Box::new(value) }),
                _ => Err("invalid assignment target".into()),
            };
        }
        Ok(expr)
    }

    /// `cond ? then : else` — right-associative, so `a ? b : c ? d : e` reads
    /// as `a ? b : (c ? d : e)`.
    fn ternary(&mut self) -> Result<Expr, String> {
        let cond = self.or_expr()?;
        if self.match_tok(&Token::Question) {
            let then_branch = self.expression()?;
            self.expect(Token::Colon, "expected ':' in '?:' expression")?;
            let else_branch = self.ternary()?;
            return Ok(Expr::Ternary { cond: Box::new(cond), then_branch: Box::new(then_branch), else_branch: Box::new(else_branch) });
        }
        Ok(cond)
    }

    fn or_expr(&mut self) -> Result<Expr, String> {
        let mut left = self.and_expr()?;
        while self.check(&Token::OrOr) || self.check(&Token::Or) {
            self.advance();
            let right = self.and_expr()?;
            left = Expr::Logical { op: "or".into(), left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn and_expr(&mut self) -> Result<Expr, String> {
        let mut left = self.equality()?;
        while self.check(&Token::AndAnd) || self.check(&Token::And) {
            self.advance();
            let right = self.equality()?;
            left = Expr::Logical { op: "and".into(), left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn equality(&mut self) -> Result<Expr, String> {
        let mut left = self.comparison()?;
        loop {
            let op = match self.peek() { Token::Eq => "==", Token::NotEq => "!=", _ => break };
            self.advance();
            let right = self.comparison()?;
            left = Expr::Binary { op: op.into(), left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn comparison(&mut self) -> Result<Expr, String> {
        let mut left = self.additive()?;
        loop {
            let op = match self.peek() { Token::Lt => "<", Token::Gt => ">", Token::LtEq => "<=", Token::GtEq => ">=", _ => break };
            self.advance();
            let right = self.additive()?;
            left = Expr::Binary { op: op.into(), left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn additive(&mut self) -> Result<Expr, String> {
        let mut left = self.multiplicative()?;
        loop {
            let op = match self.peek() { Token::Plus => "+", Token::Minus => "-", _ => break };
            self.advance();
            let right = self.multiplicative()?;
            left = Expr::Binary { op: op.into(), left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn multiplicative(&mut self) -> Result<Expr, String> {
        let mut left = self.power()?;
        loop {
            let op = match self.peek() { Token::Star => "*", Token::Slash => "/", Token::Percent => "%", Token::At => "@", _ => break };
            self.advance();
            let right = self.power()?;
            left = Expr::Binary { op: op.into(), left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn power(&mut self) -> Result<Expr, String> {
        let left = self.unary()?;
        if self.check(&Token::StarStar) {
            self.advance();
            let right = self.power()?; // right-associative
            return Ok(Expr::Binary { op: "**".into(), left: Box::new(left), right: Box::new(right) });
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Token::Minus => { self.advance(); let e = self.unary()?; Ok(Expr::Unary { op: "-".into(), expr: Box::new(e) }) }
            Token::Bang | Token::Not => { self.advance(); let e = self.unary()?; Ok(Expr::Unary { op: "!".into(), expr: Box::new(e) }) }
            _ => self.call_postfix(),
        }
    }

    fn call_postfix(&mut self) -> Result<Expr, String> {
        let mut expr = self.primary()?;
        loop {
            if self.match_tok(&Token::LParen) {
                let mut args = Vec::new();
                if !self.check(&Token::RParen) {
                    loop { args.push(self.expression()?); if !self.match_tok(&Token::Comma) { break; } }
                }
                self.expect(Token::RParen, "expected ')' after arguments")?;
                expr = Expr::Call { callee: Box::new(expr), args };
            } else if self.match_tok(&Token::LBracket) {
                let idx = self.expression()?;
                self.expect(Token::RBracket, "expected ']'")?;
                expr = Expr::Index { target: Box::new(expr), index: Box::new(idx) };
            } else if self.match_tok(&Token::Dot) {
                let name = self.ident()?;
                expr = Expr::Field { target: Box::new(expr), name };
            } else { break; }
        }
        Ok(expr)
    }

    fn primary(&mut self) -> Result<Expr, String> {
        match self.advance() {
            Token::Int(i) => Ok(Expr::Int(i)),
            Token::Float(f) => Ok(Expr::Float(f)),
            Token::Str(s) => Ok(Expr::Str(s)),
            Token::FStr(parts) => {
                let mut ip = Vec::new();
                for p in parts {
                    match p {
                        FStringPart::Lit(s) => ip.push(InterpPart::Lit(s)),
                        FStringPart::Expr(src) => {
                            let toks = crate::lexer::tokenize(&src)?;
                            let mut sub = Parser::new(toks);
                            let e = sub.expression()?;
                            ip.push(InterpPart::Expr(e));
                        }
                    }
                }
                Ok(Expr::Interp(ip))
            }
            Token::True => Ok(Expr::Bool(true)),
            Token::False => Ok(Expr::Bool(false)),
            Token::Null => Ok(Expr::Null),
            Token::Ident(name) => Ok(Expr::Ident(name)),
            Token::New => {
                let name = self.ident()?;
                self.expect(Token::LParen, "expected '(' after class name in 'new'")?;
                let mut args = Vec::new();
                if !self.check(&Token::RParen) {
                    loop { args.push(self.expression()?); if !self.match_tok(&Token::Comma) { break; } }
                }
                self.expect(Token::RParen, "expected ')'")?;
                Ok(Expr::New { class: name, args })
            }
            Token::Fn => {
                let params = self.params()?;
                let body = self.block()?;
                Ok(Expr::FuncExpr { params, body })
            }
            Token::Match => {
                // Note: if the subject itself looks like a dict literal
                // ('{'), wrap it in parens — `match {..}` is read as the
                // opening brace of the match body, same ambiguity most
                // C-like languages have with match/switch subjects.
                let subject = self.or_expr()?;
                self.expect(Token::LBrace, "expected '{' to start match body")?;
                let mut arms = Vec::new();
                let mut default = None;
                while !self.check(&Token::RBrace) {
                    let is_wildcard = matches!(self.peek(), Token::Ident(s) if s == "_");
                    if is_wildcard {
                        self.advance();
                        self.expect(Token::FatArrow, "expected '=>' after '_' in match")?;
                        default = Some(Box::new(self.expression()?));
                    } else {
                        let pattern = self.expression()?;
                        self.expect(Token::FatArrow, "expected '=>' in match arm")?;
                        let body = self.expression()?;
                        arms.push((pattern, body));
                    }
                    if !self.match_tok(&Token::Comma) { break; }
                }
                self.expect(Token::RBrace, "expected '}' to close match")?;
                Ok(Expr::Match { subject: Box::new(subject), arms, default })
            }
            Token::LParen => {
                let e = self.expression()?;
                self.expect(Token::RParen, "expected ')'")?;
                Ok(e)
            }
            Token::LBracket => {
                let mut items = Vec::new();
                if !self.check(&Token::RBracket) {
                    loop { items.push(self.expression()?); if !self.match_tok(&Token::Comma) { break; } }
                }
                self.expect(Token::RBracket, "expected ']'")?;
                Ok(Expr::List(items))
            }
            Token::LBrace => {
                let mut pairs = Vec::new();
                if !self.check(&Token::RBrace) {
                    loop {
                        let key = self.expression()?;
                        self.expect(Token::Colon, "expected ':' in dict literal")?;
                        let val = self.expression()?;
                        pairs.push((key, val));
                        if !self.match_tok(&Token::Comma) { break; }
                    }
                }
                self.expect(Token::RBrace, "expected '}'")?;
                Ok(Expr::Dict(pairs))
            }
            other => Err(format!("unexpected token {:?}", other)),
        }
    }
}

pub fn parse(tokens: Vec<Token>) -> Result<Vec<Stmt>, String> {
    Parser::new(tokens).parse_program()
}
