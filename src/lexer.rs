//! Lexer: turns K source text into a flat token stream.
//! Errors here are returned, never panicked — a typo in a user's script
//! should never crash the interpreter process.

#[derive(Debug, Clone, PartialEq)]
pub enum FStringPart {
    Lit(String),
    Expr(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // literals
    Int(i64),
    Float(f64),
    Str(String),
    FStr(Vec<FStringPart>),
    Ident(String),
    // keywords
    Let, Const, Fn, Return, If, Elif, Else, While, For, In, Break, Continue,
    True, False, Null, Class, New, Try, Catch, Throw, And, Or, Not, Import,
    As, Match,
    // operators
    Plus, Minus, Star, Slash, Percent, StarStar, At,
    Assign, PlusAssign, MinusAssign, StarAssign, SlashAssign, PercentAssign, StarStarAssign,
    Eq, NotEq, Lt, Gt, LtEq, GtEq,
    AndAnd, OrOr, Bang,
    Arrow, FatArrow, Question,
    // punctuation
    LParen, RParen, LBrace, RBrace, LBracket, RBracket,
    Comma, Colon, Semi, Dot,
    Eof,
}

pub fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut pos = 0usize;
    let mut line = 1usize;
    let n = chars.len();

    macro_rules! bump {
        () => {{
            if pos < n {
                if chars[pos] == '\n' { line += 1; }
                pos += 1;
            }
        }};
    }

    while pos < n {
        let c = chars[pos];
        match c {
            ' ' | '\t' | '\r' | '\n' => bump!(),
            '/' if peek(&chars, pos + 1) == Some('/') => {
                while pos < n && chars[pos] != '\n' { pos += 1; }
            }
            '/' if peek(&chars, pos + 1) == Some('*') => {
                pos += 2;
                while pos + 1 < n && !(chars[pos] == '*' && chars[pos + 1] == '/') {
                    if chars[pos] == '\n' { line += 1; }
                    pos += 1;
                }
                if pos + 1 >= n { return Err(format!("line {}: unterminated block comment", line)); }
                pos += 2;
            }
            '"' if peek(&chars, pos + 1) == Some('"') && peek(&chars, pos + 2) == Some('"') => {
                // Triple-quoted string: multi-line, raw (no escape processing) —
                // ends at the next """.
                pos += 3;
                let start = pos;
                while pos < n && !(chars[pos] == '"' && peek(&chars, pos + 1) == Some('"') && peek(&chars, pos + 2) == Some('"')) {
                    if chars[pos] == '\n' { line += 1; }
                    pos += 1;
                }
                if pos >= n { return Err(format!("line {}: unterminated triple-quoted string", line)); }
                let s: String = chars[start..pos].iter().collect();
                pos += 3;
                tokens.push(Token::Str(s));
            }
            '"' => {
                pos += 1;
                let s = read_string(&chars, &mut pos, line)?;
                tokens.push(Token::Str(s));
            }
            'f' if peek(&chars, pos + 1) == Some('"') => {
                pos += 2;
                let parts = read_fstring(&chars, &mut pos, line)?;
                tokens.push(Token::FStr(parts));
            }
            _ if c.is_ascii_digit() => {
                let start = pos;
                let mut is_float = false;
                while pos < n && chars[pos].is_ascii_digit() { pos += 1; }
                if pos < n && chars[pos] == '.' && pos + 1 < n && chars[pos + 1].is_ascii_digit() {
                    is_float = true;
                    pos += 1;
                    while pos < n && chars[pos].is_ascii_digit() { pos += 1; }
                }
                let text: String = chars[start..pos].iter().collect();
                if is_float {
                    tokens.push(Token::Float(text.parse().map_err(|_| format!("line {}: bad float literal '{}'", line, text))?));
                } else {
                    tokens.push(Token::Int(text.parse().map_err(|_| format!("line {}: bad integer literal '{}'", line, text))?));
                }
            }
            _ if c.is_alphabetic() || c == '_' => {
                let start = pos;
                while pos < n && (chars[pos].is_alphanumeric() || chars[pos] == '_') { pos += 1; }
                let text: String = chars[start..pos].iter().collect();
                tokens.push(match text.as_str() {
                    "let" => Token::Let, "const" => Token::Const, "fn" => Token::Fn,
                    "return" => Token::Return, "if" => Token::If, "elif" => Token::Elif,
                    "else" => Token::Else, "while" => Token::While, "for" => Token::For,
                    "in" => Token::In, "break" => Token::Break, "continue" => Token::Continue,
                    "true" => Token::True, "false" => Token::False, "null" => Token::Null,
                    "class" => Token::Class, "new" => Token::New, "try" => Token::Try,
                    "catch" => Token::Catch, "throw" => Token::Throw, "and" => Token::And,
                    "or" => Token::Or, "not" => Token::Not, "import" => Token::Import,
                    "as" => Token::As, "match" => Token::Match,
                    _ => Token::Ident(text),
                });
            }
            '+' => { if peek(&chars, pos + 1) == Some('=') { tokens.push(Token::PlusAssign); pos += 2; } else { tokens.push(Token::Plus); pos += 1; } }
            '-' => {
                if peek(&chars, pos + 1) == Some('=') { tokens.push(Token::MinusAssign); pos += 2; }
                else if peek(&chars, pos + 1) == Some('>') { tokens.push(Token::Arrow); pos += 2; }
                else { tokens.push(Token::Minus); pos += 1; }
            }
            '*' => {
                if peek(&chars, pos + 1) == Some('*') && peek(&chars, pos + 2) == Some('=') { tokens.push(Token::StarStarAssign); pos += 3; }
                else if peek(&chars, pos + 1) == Some('=') { tokens.push(Token::StarAssign); pos += 2; }
                else if peek(&chars, pos + 1) == Some('*') { tokens.push(Token::StarStar); pos += 2; }
                else { tokens.push(Token::Star); pos += 1; }
            }
            '/' => { if peek(&chars, pos + 1) == Some('=') { tokens.push(Token::SlashAssign); pos += 2; } else { tokens.push(Token::Slash); pos += 1; } }
            '%' => { if peek(&chars, pos + 1) == Some('=') { tokens.push(Token::PercentAssign); pos += 2; } else { tokens.push(Token::Percent); pos += 1; } }
            '@' => { tokens.push(Token::At); pos += 1; }
            '?' => { tokens.push(Token::Question); pos += 1; }
            '=' => {
                if peek(&chars, pos + 1) == Some('=') { tokens.push(Token::Eq); pos += 2; }
                else if peek(&chars, pos + 1) == Some('>') { tokens.push(Token::FatArrow); pos += 2; }
                else { tokens.push(Token::Assign); pos += 1; }
            }
            '!' => { if peek(&chars, pos + 1) == Some('=') { tokens.push(Token::NotEq); pos += 2; } else { tokens.push(Token::Bang); pos += 1; } }
            '<' => { if peek(&chars, pos + 1) == Some('=') { tokens.push(Token::LtEq); pos += 2; } else { tokens.push(Token::Lt); pos += 1; } }
            '>' => { if peek(&chars, pos + 1) == Some('=') { tokens.push(Token::GtEq); pos += 2; } else { tokens.push(Token::Gt); pos += 1; } }
            '&' => { if peek(&chars, pos + 1) == Some('&') { tokens.push(Token::AndAnd); pos += 2; } else { return Err(format!("line {}: unexpected '&' (did you mean '&&'?)", line)); } }
            '|' => { if peek(&chars, pos + 1) == Some('|') { tokens.push(Token::OrOr); pos += 2; } else { return Err(format!("line {}: unexpected '|' (did you mean '||'?)", line)); } }
            '(' => { tokens.push(Token::LParen); pos += 1; }
            ')' => { tokens.push(Token::RParen); pos += 1; }
            '{' => { tokens.push(Token::LBrace); pos += 1; }
            '}' => { tokens.push(Token::RBrace); pos += 1; }
            '[' => { tokens.push(Token::LBracket); pos += 1; }
            ']' => { tokens.push(Token::RBracket); pos += 1; }
            ',' => { tokens.push(Token::Comma); pos += 1; }
            ':' => { tokens.push(Token::Colon); pos += 1; }
            ';' => { tokens.push(Token::Semi); pos += 1; }
            '.' => { tokens.push(Token::Dot); pos += 1; }
            other => return Err(format!("line {}: unexpected character '{}'", line, other)),
        }
    }
    tokens.push(Token::Eof);
    Ok(tokens)
}

fn peek(chars: &[char], pos: usize) -> Option<char> { chars.get(pos).copied() }

fn read_string(chars: &[char], pos: &mut usize, line: usize) -> Result<String, String> {
    let mut s = String::new();
    while *pos < chars.len() && chars[*pos] != '"' {
        if chars[*pos] == '\\' && *pos + 1 < chars.len() {
            *pos += 1;
            s.push(match chars[*pos] { 'n' => '\n', 't' => '\t', 'r' => '\r', '"' => '"', '\\' => '\\', other => other });
            *pos += 1;
        } else {
            s.push(chars[*pos]);
            *pos += 1;
        }
    }
    if *pos >= chars.len() { return Err(format!("line {}: unterminated string literal", line)); }
    *pos += 1; // closing quote
    Ok(s)
}

fn read_fstring(chars: &[char], pos: &mut usize, line: usize) -> Result<Vec<FStringPart>, String> {
    let mut parts = Vec::new();
    let mut lit = String::new();
    while *pos < chars.len() && chars[*pos] != '"' {
        if chars[*pos] == '{' {
            if !lit.is_empty() { parts.push(FStringPart::Lit(std::mem::take(&mut lit))); }
            *pos += 1;
            let mut depth = 1;
            let mut expr_src = String::new();
            while *pos < chars.len() && depth > 0 {
                match chars[*pos] {
                    '{' => { depth += 1; expr_src.push(chars[*pos]); }
                    '}' => { depth -= 1; if depth > 0 { expr_src.push(chars[*pos]); } }
                    c => expr_src.push(c),
                }
                *pos += 1;
            }
            if depth != 0 { return Err(format!("line {}: unterminated '{{' in f-string", line)); }
            parts.push(FStringPart::Expr(expr_src));
        } else if chars[*pos] == '\\' && *pos + 1 < chars.len() {
            *pos += 1;
            lit.push(match chars[*pos] { 'n' => '\n', 't' => '\t', '"' => '"', '\\' => '\\', other => other });
            *pos += 1;
        } else {
            lit.push(chars[*pos]);
            *pos += 1;
        }
    }
    if *pos >= chars.len() { return Err(format!("line {}: unterminated f-string literal", line)); }
    if !lit.is_empty() { parts.push(FStringPart::Lit(lit)); }
    *pos += 1; // closing quote
    Ok(parts)
}
