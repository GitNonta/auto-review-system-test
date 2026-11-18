//! สูตรคะแนนภาระงานแบบกำหนดเอง (เช่น `C + pending_weight * P`)
//!
//! แดชบอร์ดใช้โมดูลนี้ทดลองสูตรอื่นก่อนแก้ `.github/auto-review.yml` จริง:
//! ผู้ใช้พิมพ์สูตร แล้วแดชบอร์ดคำนวณคะแนนของทุกคนจากตัวแปร C/P/...
//!
//! ไวยากรณ์ (เรียงจากความสำคัญต่ำไปสูง):
//! ```text
//! expr    := cond
//! cond    := or ( '?' expr ':' expr )?
//! or      := and ( '||' and )*
//! and     := cmp ( '&&' cmp )*
//! cmp     := sum ( ('==' | '!=' | '<' | '<=' | '>' | '>=') sum )?
//! sum     := term ( ('+' | '-') term )*
//! term    := unary ( ('*' | '/' | '%') unary )*
//! unary   := ('-' | '!') unary | power
//! power   := atom ( '^' unary )?          (เชื่อมขวา: 2^3^2 = 2^9)
//! atom    := number | ident | ident '(' args ')' | '(' expr ')'
//! ```
//! ค่าทุกตัวเป็น `f64`; ค่าความจริงคือ 0 = เท็จ, อื่น ๆ = จริง
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::HashMap;
use std::fmt;

/* --------------------------------- lexer --------------------------------- */

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Num(f64),
    Ident(String),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
    Question,
    Colon,
}

/// token พร้อมตำแหน่งเริ่ม (byte offset) ไว้ชี้ตำแหน่ง error
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned {
    pub tok: Tok,
    pub pos: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FormulaError {
    UnexpectedChar { pos: usize, ch: char },
    BadNumber { pos: usize, text: String },
    UnexpectedToken { pos: usize, found: String, expected: &'static str },
    UnexpectedEnd { expected: &'static str },
    UnknownVariable(String),
    UnknownFunction(String),
    Arity { name: String, expected: &'static str, got: usize },
    DivisionByZero,
    NotFinite,
    TooDeep,
}

impl fmt::Display for FormulaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use FormulaError::*;
        match self {
            UnexpectedChar { pos, ch } => write!(f, "อักขระ `{ch}` ไม่รู้จัก ที่ตำแหน่ง {pos}"),
            BadNumber { pos, text } => write!(f, "ตัวเลข `{text}` ไม่ถูกต้อง ที่ตำแหน่ง {pos}"),
            UnexpectedToken { pos, found, expected } => {
                write!(f, "เจอ `{found}` ที่ตำแหน่ง {pos} แต่ต้องการ {expected}")
            }
            UnexpectedEnd { expected } => write!(f, "สูตรจบก่อนเวลา: ต้องการ {expected}"),
            UnknownVariable(n) => write!(f, "ไม่รู้จักตัวแปร `{n}`"),
            UnknownFunction(n) => write!(f, "ไม่รู้จักฟังก์ชัน `{n}`"),
            Arity { name, expected, got } => write!(f, "`{name}` ต้องการอาร์กิวเมนต์ {expected} ตัว แต่ได้ {got}"),
            DivisionByZero => write!(f, "หารด้วยศูนย์"),
            NotFinite => write!(f, "ผลลัพธ์ไม่ใช่ตัวเลขจำกัด (NaN/∞)"),
            TooDeep => write!(f, "สูตรซ้อนวงเล็บลึกเกินไป"),
        }
    }
}

impl std::error::Error for FormulaError {}

/// operator หลายอักขระต้องอยู่ก่อนตัวที่สั้นกว่า (`<=` ก่อน `<`)
const OPS: [&str; 16] = ["<=", ">=", "==", "!=", "&&", "||", "<", ">", "+", "-", "*", "/", "%", "^", "!", "="];

pub fn tokenize(src: &str) -> Result<Vec<Spanned>, FormulaError> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    'outer: while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if c.is_ascii_digit() || (c == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.' || bytes[i] == b'_') {
                i += 1;
            }
            // exponent เช่น 1e3, 2.5E-2
            if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                let mut j = i + 1;
                if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
                    j += 1;
                }
                if j < bytes.len() && bytes[j].is_ascii_digit() {
                    i = j;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let text = &src[start..i];
            let clean: String = text.chars().filter(|&ch| ch != '_').collect();
            let n = clean.parse::<f64>().map_err(|_| FormulaError::BadNumber { pos: start, text: text.to_string() })?;
            out.push(Spanned { tok: Tok::Num(n), pos: start });
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            out.push(Spanned { tok: Tok::Ident(src[start..i].to_string()), pos: start });
            continue;
        }
        let simple = match c {
            b'(' => Some(Tok::LParen),
            b')' => Some(Tok::RParen),
            b',' => Some(Tok::Comma),
            b'?' => Some(Tok::Question),
            b':' => Some(Tok::Colon),
            _ => None,
        };
        if let Some(tok) = simple {
            out.push(Spanned { tok, pos: start });
            i += 1;
            continue;
        }
        for op in OPS {
            if src[i..].starts_with(op) {
                // `=` เดี่ยวไม่ใช่ operator ของภาษา: ผู้ใช้มักพิมพ์ผิดจาก `==`
                if op == "=" {
                    break;
                }
                out.push(Spanned { tok: Tok::Op(op), pos: start });
                i += op.len();
                continue 'outer;
            }
        }
        let ch = src[i..].chars().next().unwrap_or('?');
        return Err(FormulaError::UnexpectedChar { pos: start, ch });
    }
    Ok(out)
}

/* ---------------------------------- AST ---------------------------------- */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

impl BinOp {
    fn symbol(self) -> &'static str {
        use BinOp::*;
        match self {
            Add => "+",
            Sub => "-",
            Mul => "*",
            Div => "/",
            Rem => "%",
            Pow => "^",
            Eq => "==",
            Ne => "!=",
            Lt => "<",
            Le => "<=",
            Gt => ">",
            Ge => ">=",
            And => "&&",
            Or => "||",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    Var(String),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
}

impl fmt::Display for Expr {
    /// พิมพ์กลับแบบใส่วงเล็บครบ (ใช้แสดงว่าบอทเข้าใจสูตรว่าอย่างไร)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Num(n) => write!(f, "{n}"),
            Expr::Var(v) => write!(f, "{v}"),
            Expr::Neg(e) => write!(f, "(-{e})"),
            Expr::Not(e) => write!(f, "(!{e})"),
            Expr::Bin(op, a, b) => write!(f, "({a} {} {b})", op.symbol()),
            Expr::Call(name, args) => {
                write!(f, "{name}(")?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{a}")?;
                }
                write!(f, ")")
            }
            Expr::Cond(c, a, b) => write!(f, "({c} ? {a} : {b})"),
        }
    }
}

impl Expr {
    /// ชื่อตัวแปรทั้งหมดที่สูตรใช้ (ไม่ซ้ำ เรียงตามที่พบครั้งแรก)
    pub fn variables(&self) -> Vec<String> {
        fn walk(e: &Expr, out: &mut Vec<String>) {
            match e {
                Expr::Num(_) => {}
                Expr::Var(v) => {
                    if !out.contains(v) {
                        out.push(v.clone());
                    }
                }
                Expr::Neg(x) | Expr::Not(x) => walk(x, out),
                Expr::Bin(_, a, b) => {
                    walk(a, out);
                    walk(b, out);
                }
                Expr::Call(_, args) => args.iter().for_each(|a| walk(a, out)),
                Expr::Cond(c, a, b) => {
                    walk(c, out);
                    walk(a, out);
                    walk(b, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(self, &mut out);
        out
    }
}

/* --------------------------------- parser -------------------------------- */

/// กันสูตรที่ซ้อนลึกผิดปกติทำ stack overflow
const MAX_DEPTH: usize = 64;

struct Parser {
    toks: Vec<Spanned>,
    i: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i).map(|s| &s.tok)
    }

    fn bump(&mut self) -> Option<Spanned> {
        let t = self.toks.get(self.i).cloned();
        self.i += 1;
        t
    }

    fn eat_op(&mut self, ops: &[&'static str]) -> Option<&'static str> {
        match self.peek() {
            Some(Tok::Op(o)) if ops.contains(o) => {
                let o = *o;
                self.i += 1;
                Some(o)
            }
            _ => None,
        }
    }

    fn expect(&mut self, want: Tok, expected: &'static str) -> Result<(), FormulaError> {
        match self.bump() {
            Some(s) if s.tok == want => Ok(()),
            Some(s) => Err(FormulaError::UnexpectedToken { pos: s.pos, found: describe(&s.tok), expected }),
            None => Err(FormulaError::UnexpectedEnd { expected }),
        }
    }

    fn enter(&mut self) -> Result<(), FormulaError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(FormulaError::TooDeep);
        }
        Ok(())
    }

    fn expr(&mut self) -> Result<Expr, FormulaError> {
        self.enter()?;
        let cond = self.or()?;
        let out = if self.peek() == Some(&Tok::Question) {
            self.i += 1;
            let a = self.expr()?;
            self.expect(Tok::Colon, "`:`")?;
            let b = self.expr()?;
            Expr::Cond(Box::new(cond), Box::new(a), Box::new(b))
        } else {
            cond
        };
        self.depth -= 1;
        Ok(out)
    }

    fn or(&mut self) -> Result<Expr, FormulaError> {
        let mut lhs = self.and()?;
        while self.eat_op(&["||"]).is_some() {
            lhs = Expr::Bin(BinOp::Or, Box::new(lhs), Box::new(self.and()?));
        }
        Ok(lhs)
    }

    fn and(&mut self) -> Result<Expr, FormulaError> {
        let mut lhs = self.cmp()?;
        while self.eat_op(&["&&"]).is_some() {
            lhs = Expr::Bin(BinOp::And, Box::new(lhs), Box::new(self.cmp()?));
        }
        Ok(lhs)
    }

    fn cmp(&mut self) -> Result<Expr, FormulaError> {
        let lhs = self.sum()?;
        let op = match self.eat_op(&["==", "!=", "<", "<=", ">", ">="]) {
            Some("==") => BinOp::Eq,
            Some("!=") => BinOp::Ne,
            Some("<") => BinOp::Lt,
            Some("<=") => BinOp::Le,
            Some(">") => BinOp::Gt,
            Some(">=") => BinOp::Ge,
            _ => return Ok(lhs),
        };
        Ok(Expr::Bin(op, Box::new(lhs), Box::new(self.sum()?)))
    }

    fn sum(&mut self) -> Result<Expr, FormulaError> {
        let mut lhs = self.term()?;
        while let Some(o) = self.eat_op(&["+", "-"]) {
            let op = if o == "+" { BinOp::Add } else { BinOp::Sub };
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(self.term()?));
        }
        Ok(lhs)
    }

    fn term(&mut self) -> Result<Expr, FormulaError> {
        let mut lhs = self.unary()?;
        while let Some(o) = self.eat_op(&["*", "/", "%"]) {
            let op = match o {
                "*" => BinOp::Mul,
                "/" => BinOp::Div,
                _ => BinOp::Rem,
            };
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(self.unary()?));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr, FormulaError> {
        self.enter()?;
        let out = match self.eat_op(&["-", "!"]) {
            Some("-") => Expr::Neg(Box::new(self.unary()?)),
            Some(_) => Expr::Not(Box::new(self.unary()?)),
            None => self.power()?,
        };
        self.depth -= 1;
        Ok(out)
    }

    fn power(&mut self) -> Result<Expr, FormulaError> {
        let base = self.atom()?;
        if self.eat_op(&["^"]).is_some() {
            // เลขชี้กำลังผ่าน unary จึงได้ทั้ง 2^-1 และการเชื่อมขวา 2^3^2
            return Ok(Expr::Bin(BinOp::Pow, Box::new(base), Box::new(self.unary()?)));
        }
        Ok(base)
    }

    fn atom(&mut self) -> Result<Expr, FormulaError> {
        const WANT: &str = "ตัวเลข ตัวแปร หรือ `(`";
        let Some(s) = self.bump() else {
            return Err(FormulaError::UnexpectedEnd { expected: WANT });
        };
        match s.tok {
            Tok::Num(n) => Ok(Expr::Num(n)),
            Tok::LParen => {
                let e = self.expr()?;
                self.expect(Tok::RParen, "`)`")?;
                Ok(e)
            }
            Tok::Ident(name) => {
                if self.peek() != Some(&Tok::LParen) {
                    return Ok(Expr::Var(name));
                }
                self.i += 1;
                let mut args = Vec::new();
                if self.peek() == Some(&Tok::RParen) {
                    self.i += 1;
                    return Ok(Expr::Call(name, args));
                }
                loop {
                    args.push(self.expr()?);
                    match self.bump() {
                        Some(Spanned { tok: Tok::Comma, .. }) => continue,
                        Some(Spanned { tok: Tok::RParen, .. }) => break,
                        Some(o) => {
                            return Err(FormulaError::UnexpectedToken { pos: o.pos, found: describe(&o.tok), expected: "`,` หรือ `)`" })
                        }
                        None => return Err(FormulaError::UnexpectedEnd { expected: "`,` หรือ `)`" }),
                    }
                }
                Ok(Expr::Call(name, args))
            }
            other => Err(FormulaError::UnexpectedToken { pos: s.pos, found: describe(&other), expected: WANT }),
        }
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Num(n) => n.to_string(),
        Tok::Ident(s) => s.clone(),
        Tok::Op(o) => (*o).to_string(),
        Tok::LParen => "(".into(),
        Tok::RParen => ")".into(),
        Tok::Comma => ",".into(),
        Tok::Question => "?".into(),
        Tok::Colon => ":".into(),
    }
}

pub fn parse(src: &str) -> Result<Expr, FormulaError> {
    let toks = tokenize(src)?;
    let mut p = Parser { toks, i: 0, depth: 0 };
    let e = p.expr()?;
    if let Some(s) = p.toks.get(p.i) {
        return Err(FormulaError::UnexpectedToken { pos: s.pos, found: describe(&s.tok), expected: "จบสูตร" });
    }
    Ok(e)
}

/* ------------------------------- evaluator ------------------------------- */

fn truthy(x: f64) -> bool {
    x != 0.0
}

fn flag(b: bool) -> f64 {
    if b {
        1.0
    } else {
        0.0
    }
}

fn arity(name: &str, args: &[f64], ok: bool, expected: &'static str) -> Result<(), FormulaError> {
    if ok {
        Ok(())
    } else {
        Err(FormulaError::Arity { name: name.to_string(), expected, got: args.len() })
    }
}

fn call(name: &str, args: &[f64]) -> Result<f64, FormulaError> {
    match name {
        "min" | "max" => {
            arity(name, args, !args.is_empty(), "อย่างน้อย 1")?;
            let pick = if name == "min" { f64::min } else { f64::max };
            Ok(args[1..].iter().fold(args[0], |acc, &x| pick(acc, x)))
        }
        "abs" | "floor" | "ceil" | "round" | "sqrt" | "log2" => {
            arity(name, args, args.len() == 1, "1")?;
            let x = args[0];
            Ok(match name {
                "abs" => x.abs(),
                "floor" => x.floor(),
                "ceil" => x.ceil(),
                "round" => x.round(),
                "sqrt" => x.sqrt(),
                _ => x.log2(),
            })
        }
        "clamp" => {
            arity(name, args, args.len() == 3, "3")?;
            let (x, lo, hi) = (args[0], args[1], args[2]);
            Ok(if lo > hi { x.max(hi).min(lo) } else { x.max(lo).min(hi) })
        }
        "if" => {
            arity(name, args, args.len() == 3, "3")?;
            Ok(if truthy(args[0]) { args[1] } else { args[2] })
        }
        _ => Err(FormulaError::UnknownFunction(name.to_string())),
    }
}

pub fn eval(e: &Expr, vars: &HashMap<String, f64>) -> Result<f64, FormulaError> {
    let v = match e {
        Expr::Num(n) => *n,
        Expr::Var(name) => *vars.get(name).ok_or_else(|| FormulaError::UnknownVariable(name.clone()))?,
        Expr::Neg(x) => -eval(x, vars)?,
        Expr::Not(x) => flag(!truthy(eval(x, vars)?)),
        Expr::Cond(c, a, b) => {
            // ประเมินเฉพาะกิ่งที่เลือก: กิ่งที่ไม่ถูกเลือกหารศูนย์ได้โดยไม่ error
            if truthy(eval(c, vars)?) {
                eval(a, vars)?
            } else {
                eval(b, vars)?
            }
        }
        Expr::Bin(BinOp::And, a, b) => flag(truthy(eval(a, vars)?) && truthy(eval(b, vars)?)),
        Expr::Bin(BinOp::Or, a, b) => flag(truthy(eval(a, vars)?) || truthy(eval(b, vars)?)),
        Expr::Bin(op, a, b) => {
            let (x, y) = (eval(a, vars)?, eval(b, vars)?);
            match op {
                BinOp::Add => x + y,
                BinOp::Sub => x - y,
                BinOp::Mul => x * y,
                BinOp::Div | BinOp::Rem if y == 0.0 => return Err(FormulaError::DivisionByZero),
                BinOp::Div => x / y,
                BinOp::Rem => x % y,
                BinOp::Pow => x.powf(y),
                BinOp::Eq => flag(x == y),
                BinOp::Ne => flag(x != y),
                BinOp::Lt => flag(x < y),
                BinOp::Le => flag(x <= y),
                BinOp::Gt => flag(x > y),
                BinOp::Ge => flag(x >= y),
                BinOp::And | BinOp::Or => unreachable!("จัดการด้านบนแล้ว"),
            }
        }
        Expr::Call(name, args) => {
            if name == "if" && args.len() == 3 {
                // `if(c, a, b)` lazy เหมือน `c ? a : b`
                return eval(&Expr::Cond(Box::new(args[0].clone()), Box::new(args[1].clone()), Box::new(args[2].clone())), vars);
            }
            let vals = args.iter().map(|a| eval(a, vars)).collect::<Result<Vec<_>, _>>()?;
            call(name, &vals)?
        }
    };
    if v.is_finite() {
        Ok(v)
    } else {
        Err(FormulaError::NotFinite)
    }
}

/// สูตรที่ parse แล้วพร้อมใช้ซ้ำกับหลายคน
#[derive(Debug, Clone, PartialEq)]
pub struct Formula {
    pub source: String,
    pub ast: Expr,
}

impl Formula {
    pub fn new(src: &str) -> Result<Formula, FormulaError> {
        Ok(Formula { source: src.trim().to_string(), ast: parse(src)? })
    }

    /// สูตรเริ่มต้นของระบบ: Score = C + pending_weight × P
    pub fn default_score() -> Formula {
        Formula::new("C + pending_weight * P").expect("สูตรเริ่มต้นต้อง parse ได้")
    }

    /// ตรวจล่วงหน้าว่าทุกตัวแปรที่สูตรใช้อยู่ในชุดที่อนุญาต
    pub fn check_vars(&self, allowed: &[&str]) -> Result<(), FormulaError> {
        match self.ast.variables().into_iter().find(|v| !allowed.contains(&v.as_str())) {
            Some(v) => Err(FormulaError::UnknownVariable(v)),
            None => Ok(()),
        }
    }

    pub fn eval_pairs(&self, pairs: &[(&str, f64)]) -> Result<f64, FormulaError> {
        let vars: HashMap<String, f64> = pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        eval(&self.ast, &vars)
    }

    /// คำนวณคะแนนของหลายคน แล้วเรียงจากต่ำไปสูง (เท่ากันเรียงตามชื่อ)
    pub fn rank<'a>(&self, people: &[(&'a str, Vec<(&str, f64)>)]) -> Result<Vec<(&'a str, f64)>, FormulaError> {
        let mut out = Vec::with_capacity(people.len());
        for (who, vars) in people {
            out.push((*who, self.eval_pairs(vars)?));
        }
        out.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(b.0)));
        Ok(out)
    }
}

/* ---------------------------------- tests --------------------------------- */

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(src: &str) -> f64 {
        Formula::new(src).unwrap().eval_pairs(&[]).unwrap()
    }

    fn err(src: &str) -> FormulaError {
        match Formula::new(src) {
            Err(e) => e,
            Ok(f) => f.eval_pairs(&[("x", 1.0)]).unwrap_err(),
        }
    }

    #[test]
    fn tokenizer_positions_and_numbers() {
        let t = tokenize("a<=1_000 + .5e1").unwrap();
        let kinds: Vec<_> = t.iter().map(|s| (s.tok.clone(), s.pos)).collect();
        assert_eq!(
            kinds,
            vec![(Tok::Ident("a".into()), 0), (Tok::Op("<="), 1), (Tok::Num(1000.0), 3), (Tok::Op("+"), 9), (Tok::Num(5.0), 11)]
        );
        assert_eq!(tokenize("2e").unwrap()[1].tok, Tok::Ident("e".into()), "e ที่ไม่มีเลขตามเป็นตัวแปร");
    }

    #[test]
    fn tokenizer_rejects_unknown_chars() {
        assert_eq!(tokenize("1 # 2"), Err(FormulaError::UnexpectedChar { pos: 2, ch: '#' }));
        assert_eq!(tokenize("a = b"), Err(FormulaError::UnexpectedChar { pos: 2, ch: '=' }));
        assert_eq!(tokenize("1 ร 2"), Err(FormulaError::UnexpectedChar { pos: 2, ch: 'ร' }));
        assert!(matches!(tokenize("1.2.3"), Err(FormulaError::BadNumber { pos: 0, .. })));
    }

    #[test]
    fn precedence_and_associativity() {
        assert_eq!(ev("1 + 2 * 3"), 7.0);
        assert_eq!(ev("(1 + 2) * 3"), 9.0);
        assert_eq!(ev("10 - 4 - 3"), 3.0, "ลบเชื่อมซ้าย");
        assert_eq!(ev("2 ^ 3 ^ 2"), 512.0, "ยกกำลังเชื่อมขวา");
        assert_eq!(ev("-2 ^ 2"), -4.0, "ลบนำหน้าต่ำกว่ายกกำลัง");
        assert_eq!(ev("2 ^ -1"), 0.5);
        assert_eq!(ev("7 % 4 * 2"), 6.0);
        assert_eq!(ev("1 + 1 == 2 && 3 > 2"), 1.0);
    }

    #[test]
    fn display_shows_parenthesized_tree() {
        let f = Formula::new("a + b * -c ? max(a, 2) : !b").unwrap();
        assert_eq!(f.ast.to_string(), "((a + (b * (-c))) ? max(a, 2) : (!b))");
        assert_eq!(Formula::new(" x ").unwrap().source, "x");
    }

    #[test]
    fn logic_short_circuit_and_conditional_laziness() {
        assert_eq!(ev("0 && 1 / 0"), 0.0);
        assert_eq!(ev("1 || 1 / 0"), 1.0);
        assert_eq!(ev("1 ? 5 : 1 / 0"), 5.0);
        assert_eq!(ev("if(0, 1 / 0, 9)"), 9.0);
        assert_eq!(ev("!0 + !3"), 1.0);
        assert_eq!(ev("0 ? 1 : 0 ? 2 : 3"), 3.0, "ternary ซ้อนเชื่อมขวา");
    }

    #[test]
    fn builtin_functions() {
        assert_eq!(ev("min(4, 2, 8)"), 2.0);
        assert_eq!(ev("max(4)"), 4.0);
        assert_eq!(ev("abs(-3) + floor(2.7) + ceil(2.1) + round(2.5)"), 3.0 + 2.0 + 3.0 + 3.0);
        assert_eq!(ev("sqrt(81) + log2(8)"), 12.0);
        assert_eq!(ev("clamp(15, 0, 10)"), 10.0);
        assert_eq!(ev("clamp(-1, 10, 0)"), 0.0, "ขอบสลับกันยังบีบได้");
    }

    #[test]
    fn function_errors() {
        assert_eq!(err("nope(1)"), FormulaError::UnknownFunction("nope".into()));
        assert_eq!(err("min()"), FormulaError::Arity { name: "min".into(), expected: "อย่างน้อย 1", got: 0 });
        assert_eq!(err("abs(1, 2)"), FormulaError::Arity { name: "abs".into(), expected: "1", got: 2 });
        assert_eq!(err("clamp(1)").to_string(), "`clamp` ต้องการอาร์กิวเมนต์ 3 ตัว แต่ได้ 1");
    }

    #[test]
    fn evaluation_errors() {
        assert_eq!(err("1 / (x - 1)"), FormulaError::DivisionByZero);
        assert_eq!(err("5 % 0"), FormulaError::DivisionByZero);
        assert_eq!(err("sqrt(-1)"), FormulaError::NotFinite);
        assert_eq!(err("10 ^ 400"), FormulaError::NotFinite);
        assert_eq!(err("y + 1"), FormulaError::UnknownVariable("y".into()));
    }

    #[test]
    fn syntax_errors_point_at_position() {
        assert_eq!(err("1 +"), FormulaError::UnexpectedEnd { expected: "ตัวเลข ตัวแปร หรือ `(`" });
        assert_eq!(err("(1 + 2"), FormulaError::UnexpectedEnd { expected: "`)`" });
        assert_eq!(err("1 2"), FormulaError::UnexpectedToken { pos: 2, found: "2".into(), expected: "จบสูตร" });
        assert_eq!(err("max(1 2)"), FormulaError::UnexpectedToken { pos: 6, found: "2".into(), expected: "`,` หรือ `)`" });
        assert_eq!(err("1 ? 2"), FormulaError::UnexpectedEnd { expected: "`:`" });
        assert!(err(") + 1").to_string().contains("ตำแหน่ง 0"));
        assert_eq!(err("1 < 2 < 3"), FormulaError::UnexpectedToken { pos: 6, found: "<".into(), expected: "จบสูตร" }, "เปรียบเทียบต่อกันไม่ได้");
    }

    #[test]
    fn deep_nesting_is_rejected_not_crashing() {
        let deep = format!("{}1{}", "(".repeat(200), ")".repeat(200));
        assert_eq!(err(&deep), FormulaError::TooDeep);
        let negs = format!("{}1", "-".repeat(200));
        assert_eq!(err(&negs), FormulaError::TooDeep);
        let ok = format!("{}1{}", "(".repeat(20), ")".repeat(20));
        assert_eq!(ev(&ok), 1.0);
    }

    #[test]
    fn variables_are_listed_once_in_order() {
        let f = Formula::new("P * w + C - P / max(C, k)").unwrap();
        assert_eq!(f.ast.variables(), vec!["P", "w", "C", "k"]);
        assert!(f.check_vars(&["C", "P", "w", "k"]).is_ok());
        assert_eq!(f.check_vars(&["C", "P"]), Err(FormulaError::UnknownVariable("w".into())));
        assert!(Formula::new("max()").unwrap().ast.variables().is_empty());
    }

    #[test]
    fn default_score_matches_auto_review_config() {
        let f = Formula::default_score();
        let s = f.eval_pairs(&[("C", 79.0), ("P", 10.0), ("pending_weight", 2.0)]).unwrap();
        assert_eq!(s, 99.0);
        assert_eq!(f.ast.to_string(), "(C + (pending_weight * P))");
    }

    #[test]
    fn rank_orders_people_by_score_then_name() {
        let f = Formula::new("C + 2 * P").unwrap();
        let ranked = f
            .rank(&[
                ("themynon2", vec![("C", 0.0), ("P", 60.0)]),
                ("nontawat", vec![("C", 120.0), ("P", 0.0)]),
                ("alice", vec![("C", 20.0), ("P", 10.0)]),
            ])
            .unwrap();
        assert_eq!(ranked, vec![("alice", 40.0), ("nontawat", 120.0), ("themynon2", 120.0)]);
        let bad = f.rank(&[("x", vec![("C", 1.0)])]);
        assert_eq!(bad, Err(FormulaError::UnknownVariable("P".into())));
    }

    #[test]
    fn capped_pending_formula_example() {
        // ตัวอย่างสูตรทางเลือก: งานค้างเกินเพดานนับเพิ่มเป็นสองเท่า
        let f = Formula::new("C + w * (P > cap ? cap + 2 * (P - cap) : P)").unwrap();
        let at = |p: f64| f.eval_pairs(&[("C", 100.0), ("w", 2.0), ("P", p), ("cap", 800.0)]).unwrap();
        assert_eq!(at(300.0), 700.0);
        assert_eq!(at(800.0), 1700.0);
        assert_eq!(at(900.0), 100.0 + 2.0 * 1000.0);
    }

    #[test]
    fn error_display_is_human_readable() {
        assert_eq!(FormulaError::DivisionByZero.to_string(), "หารด้วยศูนย์");
        assert_eq!(FormulaError::TooDeep.to_string(), "สูตรซ้อนวงเล็บลึกเกินไป");
        assert!(FormulaError::UnexpectedChar { pos: 3, ch: '$' }.to_string().contains('$'));
        assert!(FormulaError::BadNumber { pos: 0, text: "1..2".into() }.to_string().contains("1..2"));
        assert!(FormulaError::UnknownFunction("f".into()).to_string().contains("`f`"));
        assert!(FormulaError::NotFinite.to_string().contains("NaN"));
    }
}
