/// Simple expression parser and evaluator for scalar functions: f(x,y,z) -> scalar
/// Supports arithmetic, standard math functions, and constants.

use std::f64::consts;

#[derive(Clone)]
enum Expr {
    Var(char),                    // x, y, z
    Num(f64),                     // 3.14
    BinOp(Box<Expr>, Op, Box<Expr>), // a + b
    UnOp(UnOp, Box<Expr>),        // -a
    Call(Func, Vec<Expr>),        // sin(x)
}

#[derive(Clone, Copy)]
enum Op {
    Add, Sub, Mul, Div, Pow,
    // Comparison
    Lt, Gt, Le, Ge, Eq, Ne,
    // Logical
    And, Or,
}

#[derive(Clone, Copy)]
enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Copy)]
enum Func {
    // Essential
    Abs, Sqrt, Min, Max, Clamp,
    Sin, Cos, Tan, Asin, Acos, Atan, Atan2,
    Floor, Ceil, Round,
    // Nice to have
    Exp, Ln, Log10, Log2,
    Fract, Mod, Sign,
    // Hyperbolic
    Sinh, Cosh, Tanh,
    // Graphics/utility
    Step, Smoothstep, Mix, Lerp, Length, Pow,
}

/// Parse and compile an expression string.
pub fn parse(s: &str) -> Result<Box<dyn Fn(f64, f64, f64) -> f64>, String> {
    let expr = Parser::new(s).parse()?;
    Ok(Box::new(move |x, y, z| eval(&expr, x, y, z)))
}

fn eval(expr: &Expr, x: f64, y: f64, z: f64) -> f64 {
    match expr {
        Expr::Var('x') => x,
        Expr::Var('y') => y,
        Expr::Var('z') => z,
        Expr::Var(_) => 0.0,
        Expr::Num(n) => *n,
        Expr::BinOp(l, op, r) => {
            let lv = eval(l, x, y, z);
            let rv = eval(r, x, y, z);
            match op {
                Op::Add => lv + rv,
                Op::Sub => lv - rv,
                Op::Mul => lv * rv,
                Op::Div => lv / rv,
                Op::Pow => lv.powf(rv),
                // Comparison operators return 0.0 or 1.0
                Op::Lt => if lv < rv { 1.0 } else { 0.0 },
                Op::Gt => if lv > rv { 1.0 } else { 0.0 },
                Op::Le => if lv <= rv { 1.0 } else { 0.0 },
                Op::Ge => if lv >= rv { 1.0 } else { 0.0 },
                Op::Eq => if (lv - rv).abs() < 1e-10 { 1.0 } else { 0.0 },
                Op::Ne => if (lv - rv).abs() >= 1e-10 { 1.0 } else { 0.0 },
                // Logical operators (non-zero = true)
                Op::And => if lv != 0.0 && rv != 0.0 { 1.0 } else { 0.0 },
                Op::Or => if lv != 0.0 || rv != 0.0 { 1.0 } else { 0.0 },
            }
        }
        Expr::UnOp(op, e) => {
            let v = eval(e, x, y, z);
            match op {
                UnOp::Neg => -v,
                UnOp::Not => if v == 0.0 { 1.0 } else { 0.0 },
            }
        }
        Expr::Call(func, args) => {
            let vals: Vec<f64> = args.iter().map(|a| eval(a, x, y, z)).collect();
            match func {
                Func::Abs => vals[0].abs(),
                Func::Sqrt => vals[0].sqrt(),
                Func::Min => vals[0].min(vals[1]),
                Func::Max => vals[0].max(vals[1]),
                Func::Clamp => vals[0].clamp(vals[1], vals[2]),
                Func::Sin => vals[0].sin(),
                Func::Cos => vals[0].cos(),
                Func::Tan => vals[0].tan(),
                Func::Asin => vals[0].asin(),
                Func::Acos => vals[0].acos(),
                Func::Atan => vals[0].atan(),
                Func::Atan2 => vals[0].atan2(vals[1]),
                Func::Floor => vals[0].floor(),
                Func::Ceil => vals[0].ceil(),
                Func::Round => vals[0].round(),
                Func::Exp => vals[0].exp(),
                Func::Ln => vals[0].ln(),
                Func::Log10 => vals[0].log10(),
                Func::Log2 => vals[0].log2(),
                Func::Fract => vals[0].fract(),
                Func::Mod => vals[0] % vals[1],
                Func::Sign => vals[0].signum(),
                Func::Sinh => vals[0].sinh(),
                Func::Cosh => vals[0].cosh(),
                Func::Tanh => vals[0].tanh(),
                // Graphics/utility functions
                Func::Step => if vals[0] < vals[1] { 0.0 } else { 1.0 },
                Func::Smoothstep => {
                    let edge0 = vals[0];
                    let edge1 = vals[1];
                    let x = vals[2];
                    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
                    t * t * (3.0 - 2.0 * t)
                }
                Func::Mix | Func::Lerp => vals[0] + vals[2] * (vals[1] - vals[0]),
                Func::Length => (vals[0] * vals[0] + vals[1] * vals[1] + vals[2] * vals[2]).sqrt(),
                Func::Pow => vals[0].powf(vals[1]),
            }
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Lexer
// ────────────────────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
enum Token<'a> {
    Num(f64),
    Ident(&'a str),
    Plus, Minus, Star, Slash, Caret,
    LParen, RParen, Comma,
    // Comparison operators
    Lt, Gt, Le, Ge, Eq, Ne,
    // Logical operators
    And, Or, Not,
    Eof,
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(s: &'a str) -> Self {
        Self { src: s, pos: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.src.as_bytes().get(self.pos).copied()
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.advance();
        }
    }

    fn read_number(&mut self) -> f64 {
        let start = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9' | b'.')) {
            self.advance();
        }
        self.src[start..self.pos].parse().unwrap_or(0.0)
    }

    fn read_ident(&mut self) -> &'a str {
        let start = self.pos;
        while matches!(self.peek(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')) {
            self.advance();
        }
        &self.src[start..self.pos]
    }

    fn next(&mut self) -> Token<'a> {
        self.skip_whitespace();
        match self.peek() {
            None => Token::Eof,
            Some(b'+') => { self.advance(); Token::Plus }
            Some(b'-') => { self.advance(); Token::Minus }
            Some(b'*') => { self.advance(); Token::Star }
            Some(b'/') => { self.advance(); Token::Slash }
            Some(b'^') => { self.advance(); Token::Caret }
            Some(b'(') => { self.advance(); Token::LParen }
            Some(b')') => { self.advance(); Token::RParen }
            Some(b',') => { self.advance(); Token::Comma }
            Some(b'<') => {
                self.advance();
                if matches!(self.peek(), Some(b'=')) {
                    self.advance();
                    Token::Le
                } else {
                    Token::Lt
                }
            }
            Some(b'>') => {
                self.advance();
                if matches!(self.peek(), Some(b'=')) {
                    self.advance();
                    Token::Ge
                } else {
                    Token::Gt
                }
            }
            Some(b'=') => {
                self.advance();
                if matches!(self.peek(), Some(b'=')) {
                    self.advance();
                    Token::Eq
                } else {
                    let start = self.pos - 1;
                    Token::Ident(&self.src[start..self.pos])
                }
            }
            Some(b'!') => {
                self.advance();
                if matches!(self.peek(), Some(b'=')) {
                    self.advance();
                    Token::Ne
                } else {
                    Token::Not
                }
            }
            Some(b'&') => {
                self.advance();
                if matches!(self.peek(), Some(b'&')) {
                    self.advance();
                    Token::And
                } else {
                    let start = self.pos - 1;
                    Token::Ident(&self.src[start..self.pos])
                }
            }
            Some(b'|') => {
                self.advance();
                if matches!(self.peek(), Some(b'|')) {
                    self.advance();
                    Token::Or
                } else {
                    let start = self.pos - 1;
                    Token::Ident(&self.src[start..self.pos])
                }
            }
            Some(b'0'..=b'9' | b'.') => Token::Num(self.read_number()),
            Some(b'a'..=b'z' | b'A'..=b'Z' | b'_') => Token::Ident(self.read_ident()),
            Some(_) => {
                let start = self.pos;
                self.advance();
                Token::Ident(&self.src[start..self.pos])
            }
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Parser (recursive descent)
// ────────────────────────────────────────────────────────────────────────────

struct Parser<'a> {
    lexer: Lexer<'a>,
    current: Token<'a>,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        let mut lexer = Lexer::new(s);
        let current = lexer.next();
        Self { lexer, current }
    }

    fn advance(&mut self) {
        self.current = self.lexer.next();
    }

    fn parse(&mut self) -> Result<Expr, String> {
        let expr = self.expr()?;
        if self.current != Token::Eof {
            return Err("Unexpected token after expression".into());
        }
        Ok(expr)
    }

    // expr = logical_or
    fn expr(&mut self) -> Result<Expr, String> {
        self.logical_or()
    }

    // logical_or = logical_and ('||' logical_and)*
    fn logical_or(&mut self) -> Result<Expr, String> {
        let mut left = self.logical_and()?;
        while matches!(&self.current, Token::Or) {
            self.advance();
            let right = self.logical_and()?;
            left = Expr::BinOp(Box::new(left), Op::Or, Box::new(right));
        }
        Ok(left)
    }

    // logical_and = comparison ('&&' comparison)*
    fn logical_and(&mut self) -> Result<Expr, String> {
        let mut left = self.comparison()?;
        while matches!(&self.current, Token::And) {
            self.advance();
            let right = self.comparison()?;
            left = Expr::BinOp(Box::new(left), Op::And, Box::new(right));
        }
        Ok(left)
    }

    // comparison = addition (('<' | '>' | '<=' | '>=' | '==' | '!=') addition)*
    fn comparison(&mut self) -> Result<Expr, String> {
        let mut left = self.addition()?;
        loop {
            let op = match &self.current {
                Token::Lt => Op::Lt,
                Token::Gt => Op::Gt,
                Token::Le => Op::Le,
                Token::Ge => Op::Ge,
                Token::Eq => Op::Eq,
                Token::Ne => Op::Ne,
                _ => break,
            };
            self.advance();
            let right = self.addition()?;
            left = Expr::BinOp(Box::new(left), op, Box::new(right));
        }
        Ok(left)
    }

    // addition = term (('+' | '-') term)*
    fn addition(&mut self) -> Result<Expr, String> {
        let mut left = self.term()?;
        loop {
            match &self.current {
                Token::Plus => {
                    self.advance();
                    let right = self.term()?;
                    left = Expr::BinOp(Box::new(left), Op::Add, Box::new(right));
                }
                Token::Minus => {
                    self.advance();
                    let right = self.term()?;
                    left = Expr::BinOp(Box::new(left), Op::Sub, Box::new(right));
                }
                _ => break,
            }
        }
        Ok(left)
    }

    // term = factor (('*' | '/') factor)*
    fn term(&mut self) -> Result<Expr, String> {
        let mut left = self.factor()?;
        loop {
            match &self.current {
                Token::Star => {
                    self.advance();
                    let right = self.factor()?;
                    left = Expr::BinOp(Box::new(left), Op::Mul, Box::new(right));
                }
                Token::Slash => {
                    self.advance();
                    let right = self.factor()?;
                    left = Expr::BinOp(Box::new(left), Op::Div, Box::new(right));
                }
                _ => break,
            }
        }
        Ok(left)
    }

    // factor = unary ('^' factor)*  (right-associative)
    fn factor(&mut self) -> Result<Expr, String> {
        let mut left = self.unary()?;
        if matches!(&self.current, Token::Caret) {
            self.advance();
            let right = self.factor()?; // right-associative
            left = Expr::BinOp(Box::new(left), Op::Pow, Box::new(right));
        }
        Ok(left)
    }

    // unary = '-' unary | '!' unary | primary
    fn unary(&mut self) -> Result<Expr, String> {
        match &self.current {
            Token::Minus => {
                self.advance();
                let expr = self.unary()?;
                Ok(Expr::UnOp(UnOp::Neg, Box::new(expr)))
            }
            Token::Not => {
                self.advance();
                let expr = self.unary()?;
                Ok(Expr::UnOp(UnOp::Not, Box::new(expr)))
            }
            _ => self.primary()
        }
    }

    // primary = number | ident | ident '(' args ')' | '(' expr ')'
    fn primary(&mut self) -> Result<Expr, String> {
        match self.current.clone() {
            Token::Num(n) => {
                self.advance();
                Ok(Expr::Num(n))
            }
            Token::Ident(name) => {
                self.advance();
                // Check for function call
                if matches!(&self.current, Token::LParen) {
                    self.advance();
                    let args = self.args()?;
                    if !matches!(&self.current, Token::RParen) {
                        return Err("Expected ')'".into());
                    }
                    self.advance();
                    let func = match name {
                        "abs" => Func::Abs,
                        "sqrt" => Func::Sqrt,
                        "min" => Func::Min,
                        "max" => Func::Max,
                        "clamp" => Func::Clamp,
                        "sin" => Func::Sin,
                        "cos" => Func::Cos,
                        "tan" => Func::Tan,
                        "asin" => Func::Asin,
                        "acos" => Func::Acos,
                        "atan" => Func::Atan,
                        "atan2" => Func::Atan2,
                        "floor" => Func::Floor,
                        "ceil" => Func::Ceil,
                        "round" => Func::Round,
                        "exp" => Func::Exp,
                        "ln" => Func::Ln,
                        "log10" => Func::Log10,
                        "log2" => Func::Log2,
                        "fract" => Func::Fract,
                        "mod" => Func::Mod,
                        "sign" => Func::Sign,
                        "sinh" => Func::Sinh,
                        "cosh" => Func::Cosh,
                        "tanh" => Func::Tanh,
                        "step" => Func::Step,
                        "smoothstep" => Func::Smoothstep,
                        "mix" => Func::Mix,
                        "lerp" => Func::Lerp,
                        "length" => Func::Length,
                        "pow" => Func::Pow,
                        _ => return Err("unknown function".into()),
                    };
                    Ok(Expr::Call(func, args))
                } else {
                    // Variable or constant
                    match name {
                        "x" => Ok(Expr::Var('x')),
                        "y" => Ok(Expr::Var('y')),
                        "z" => Ok(Expr::Var('z')),
                        "pi" => Ok(Expr::Num(consts::PI)),
                        "e" => Ok(Expr::Num(consts::E)),
                        "tau" => Ok(Expr::Num(consts::TAU)),
                        _ => Err("unknown variable".into()),
                    }
                }
            }
            Token::LParen => {
                self.advance();
                let expr = self.expr()?;
                if !matches!(&self.current, Token::RParen) {
                    return Err("Expected ')'".into());
                }
                self.advance();
                Ok(expr)
            }
            _ => Err("Unexpected token".into()),
        }
    }

    // args = expr (',' expr)*
    fn args(&mut self) -> Result<Vec<Expr>, String> {
        let mut args = vec![];
        if matches!(&self.current, Token::RParen) {
            return Ok(args);
        }
        args.push(self.expr()?);
        while matches!(&self.current, Token::Comma) {
            self.advance();
            args.push(self.expr()?);
        }
        Ok(args)
    }
}
