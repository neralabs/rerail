//! Bitlex source to statements.
//!
//! Lines are statements. A statement continues on the next line when a bracket is open, when the
//! line ends with an operator, a comma or `=`, or when the next line starts with `|`, `&`, `^` or
//! a literal.

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Str(Vec<u8>),
    Int(u64),
    UPlus(u32),
    P(&'static str),
    Nl,
}

#[derive(Clone, Debug)]
pub enum Arg {
    E(Expr),
    N(u64),
    S(Vec<u8>),
}

#[derive(Clone, Debug)]
pub enum Expr {
    Name(String),
    Empty,
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Xor(Box<Expr>, Box<Expr>),
    Call(String, Vec<Arg>),
    /// `T.in(A)`, `T.on(A -> B)`, `T.final(A)`, `T.reached(A)`, `T.eq(k|C)`, `T.ge(k)`.
    /// `C.frame(M)`, `C.since(M)` for a count `C`.
    Test { target: String, method: String, from: Vec<String>, to: Vec<String>, k: Option<u64>, other: Option<String>, arg: Option<Box<Expr>> },
}

#[derive(Clone, Debug)]
pub enum UClass {
    Table(String, String),
    Points(Vec<(u32, u32)>),
}

#[derive(Clone, Debug, Default)]
pub struct MachineDef {
    pub events: Vec<(String, Expr)>,
    pub states: Vec<String>,
    pub groups: Vec<(String, Vec<String>)>,
    pub start: String,
    pub finals: Vec<String>,
    /// (state, symbols, target); a symbol `any` matches every symbol not listed for the state.
    pub rules: Vec<(String, Vec<String>, String)>,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Lexer(String),
    Class(String, Vec<(u32, u32)>),
    Mask(String, Expr),
    Reflect { name: String, a: Expr, an: String, b: Expr, bn: String, reset: Option<Expr>, start_lane: bool },
    /// A count with a floor at 0 and a declared bound: above `max`, an error (a rejection
    /// inside `hypotheses`), and the count stays at `max + 1`.
    Count { name: String, up: Expr, down: Expr, reset: Option<Expr>, max: u32 },
    Machine(String, MachineDef),
    Block { name: String, params: Vec<String>, body: Vec<Stmt> },
    Inst { name: String, template: String, args: Vec<Expr> },
    Hyp { name: String, split: Expr, lanes: Vec<String>, body: Vec<Stmt> },
    Reject(String, Expr),
    Exit(String),
    /// At most `n` rounds. `until settled`: a backend may stop once a round changes no carry;
    /// without it, every round runs. Either way the values are the same.
    Rounds { n: usize, settle: bool, body: Vec<Stmt> },
    Carry(String, Expr),
    Next(String, Expr),
    Ucode { over: Expr, classes: Vec<(String, UClass)> },
    Token { kind: String, start: Expr, through: Option<Expr> },
    Munch { kind: String, over: Expr, lits: Vec<(Vec<u8>, Option<String>)> },
    Keywords { kind: String, words: Vec<(Vec<u8>, Option<String>)>, soft: Vec<Vec<u8>>, required: bool },
    Skip(Expr),
    Error(String, Expr),
}

impl Stmt {
    /// Names this statement defines, recursively through `rounds` and `hypotheses` (which do not
    /// open a scope). Blocks and instances define their own namespaces.
    pub fn defines(&self, out: &mut Vec<String>) {
        match self {
            Stmt::Class(n, _) | Stmt::Mask(n, _) | Stmt::Machine(n, _) | Stmt::Carry(n, _) => out.push(n.clone()),
            Stmt::Reflect { name, .. } | Stmt::Count { name, .. } => out.push(name.clone()),
            Stmt::Rounds { body, .. } | Stmt::Hyp { body, .. } => body.iter().for_each(|s| s.defines(out)),
            Stmt::Ucode { classes, .. } => classes.iter().for_each(|c| out.push(c.0.clone())),
            _ => {}
        }
    }
}

const PUNCT: [&str; 18] = ["->", "..", "(", ")", "[", "]", "{", "}", ",", "=", "|", "&", "^", "~", ".", "*", ":", "+"];

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let b = src.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    let mut line = 1;
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            out.push(Tok::Nl);
            line += 1;
            i += 1;
        } else if c == b' ' || c == b'\t' || c == b'\r' {
            i += 1;
        } else if c == b'#' {
            while i < b.len() && b[i] != b'\n' {
                i += 1
            }
        } else if c == b'\'' || c == b'"' {
            let q = c;
            i += 1;
            let mut s = vec![];
            while i < b.len() && b[i] != q {
                if b[i] == b'\\' && i + 1 < b.len() {
                    i += 1;
                    s.push(match b[i] {
                        b'n' => b'\n',
                        b't' => b'\t',
                        b'r' => b'\r',
                        b'f' => 0x0c,
                        b'0' => 0,
                        x => x,
                    });
                } else {
                    s.push(b[i])
                }
                i += 1;
            }
            if i >= b.len() {
                return Err(format!("line {line}: unterminated string"));
            }
            i += 1;
            out.push(Tok::Str(s));
        } else if c == b'U' && b.get(i + 1) == Some(&b'+') {
            let mut j = i + 2;
            while j < b.len() && b[j].is_ascii_hexdigit() {
                j += 1
            }
            out.push(Tok::UPlus(u32::from_str_radix(&src[i + 2..j], 16).map_err(|e| format!("line {line}: {e}"))?));
            i = j;
        } else if c.is_ascii_digit() {
            let mut j = i;
            let v = if src[i..].starts_with("0x") {
                j += 2;
                while j < b.len() && b[j].is_ascii_hexdigit() {
                    j += 1
                }
                u64::from_str_radix(&src[i + 2..j], 16)
            } else {
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1
                }
                src[i..j].parse()
            };
            out.push(Tok::Int(v.map_err(|e| format!("line {line}: {e}"))?));
            i = j;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1
            }
            out.push(Tok::Ident(src[i..j].to_string()));
            i = j;
        } else if let Some(p) = PUNCT.iter().find(|p| src[i..].starts_with(**p)) {
            out.push(Tok::P(p));
            i += p.len();
        } else {
            return Err(format!("line {line}: unexpected {:?}", c as char));
        }
    }
    // Join continued lines.
    let mut joined: Vec<Tok> = vec![];
    let mut depth = 0i32;
    for (k, t) in out.iter().enumerate() {
        match t {
            Tok::P("(") | Tok::P("[") => depth += 1,
            Tok::P(")") | Tok::P("]") => depth -= 1,
            _ => {}
        }
        if *t == Tok::Nl {
            let prev_cont = matches!(joined.last(), Some(Tok::P("|" | "&" | "^" | "~" | "," | "=" | "->" | "(")));
            let next = out[k + 1..].iter().find(|t| **t != Tok::Nl);
            let next_cont = matches!(next, Some(Tok::P("|" | "&" | "^")) | Some(Tok::Str(_)) | Some(Tok::Int(_)) | Some(Tok::UPlus(_)));
            if depth > 0 || prev_cont || next_cont || joined.last() == Some(&Tok::Nl) || joined.is_empty() {
                continue;
            }
        }
        joined.push(t.clone());
    }
    Ok(joined)
}

struct P {
    t: Vec<Tok>,
    i: usize,
}

type R<T> = Result<T, String>;

impl P {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn peek_at(&self, k: usize) -> Option<&Tok> {
        self.t.get(self.i + k)
    }
    fn err<T>(&self, what: &str) -> R<T> {
        let ctx: Vec<String> = self.t[self.i.saturating_sub(3)..(self.i + 4).min(self.t.len())].iter().map(|t| format!("{t:?}")).collect();
        Err(format!("expected {what} at token {} near {}", self.i, ctx.join(" ")))
    }
    fn is_p(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok::P(q)) if *q == p)
    }
    fn is_kw(&self, k: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(q)) if q == k)
    }
    fn eat_p(&mut self, p: &str) -> bool {
        if self.is_p(p) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn eat_kw(&mut self, k: &str) -> bool {
        if self.is_kw(k) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn want_p(&mut self, p: &str) -> R<()> {
        if self.eat_p(p) {
            Ok(())
        } else {
            self.err(p)
        }
    }
    fn want_kw(&mut self, k: &str) -> R<()> {
        if self.eat_kw(k) {
            Ok(())
        } else {
            self.err(k)
        }
    }
    fn ident(&mut self) -> R<String> {
        match self.peek() {
            Some(Tok::Ident(s)) => {
                let s = s.clone();
                self.i += 1;
                Ok(s)
            }
            _ => self.err("a name"),
        }
    }
    fn string(&mut self) -> R<Vec<u8>> {
        match self.peek() {
            Some(Tok::Str(s)) => {
                let s = s.clone();
                self.i += 1;
                Ok(s)
            }
            _ => self.err("a string"),
        }
    }
    fn int(&mut self) -> R<u64> {
        match self.peek() {
            Some(Tok::Int(v)) => {
                let v = *v;
                self.i += 1;
                Ok(v)
            }
            _ => self.err("a number"),
        }
    }
    fn nls(&mut self) {
        while self.peek() == Some(&Tok::Nl) {
            self.i += 1
        }
    }
    fn end_stmt(&mut self) -> R<()> {
        match self.peek() {
            None | Some(Tok::Nl) => {
                self.nls();
                Ok(())
            }
            Some(Tok::P("}")) => Ok(()),
            _ => self.err("end of line"),
        }
    }
    fn kind(&mut self) -> R<String> {
        match self.peek() {
            Some(Tok::Str(s)) => {
                let s = String::from_utf8_lossy(s).into_owned();
                self.i += 1;
                Ok(s)
            }
            _ => self.ident(),
        }
    }

    // --- expressions ---
    fn expr(&mut self) -> R<Expr> {
        let mut l = self.xor()?;
        while self.eat_p("|") {
            l = Expr::Or(Box::new(l), Box::new(self.xor()?));
        }
        Ok(l)
    }
    fn xor(&mut self) -> R<Expr> {
        let mut l = self.and()?;
        while self.eat_p("^") {
            l = Expr::Xor(Box::new(l), Box::new(self.and()?));
        }
        Ok(l)
    }
    fn and(&mut self) -> R<Expr> {
        let mut l = self.unary()?;
        while self.eat_p("&") {
            l = Expr::And(Box::new(l), Box::new(self.unary()?));
        }
        Ok(l)
    }
    fn unary(&mut self) -> R<Expr> {
        if self.eat_p("~") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        if self.eat_p("(") {
            let e = self.expr()?;
            self.want_p(")")?;
            return Ok(e);
        }
        if self.eat_kw("empty") {
            return Ok(Expr::Empty);
        }
        let mut parts = vec![self.ident()?];
        if self.is_p("*") && matches!(self.peek_at(1), Some(Tok::P("."))) {
            self.i += 1;
            let l = parts.pop().unwrap();
            parts.push(l + "*");
        }
        while self.is_p(".") {
            self.i += 1;
            parts.push(self.ident()?);
        }
        if !self.is_p("(") {
            return Ok(Expr::Name(parts.join(".")));
        }
        let last = parts.last().unwrap().clone();
        if parts.len() > 1 && ["in", "on", "final", "reached", "eq", "ge", "frame", "since"].contains(&last.as_str()) {
            parts.pop();
            let target = parts.join(".");
            self.want_p("(")?;
            let (mut from, mut to, mut k, mut other, mut arg) = (vec![], vec![], None, None, None);
            if last == "frame" || last == "since" {
                arg = Some(Box::new(self.expr()?));
            } else if last == "eq" || last == "ge" {
                match self.peek() {
                    Some(Tok::Int(_)) => k = Some(self.int()?),
                    _ => other = Some(self.ident()?),
                }
            } else {
                from = self.states()?;
                if self.eat_p("->") {
                    to = self.states()?;
                }
            }
            self.want_p(")")?;
            return Ok(Expr::Test { target, method: last, from, to, k, other, arg });
        }
        if parts.len() > 1 {
            return self.err("a test (in, on, final, reached, eq, ge)");
        }
        self.want_p("(")?;
        let mut args = vec![];
        loop {
            match self.peek() {
                Some(Tok::Int(_)) => args.push(Arg::N(self.int()?)),
                Some(Tok::Str(_)) => args.push(Arg::S(self.string()?)),
                _ => args.push(Arg::E(self.expr()?)),
            }
            if !self.eat_p(",") {
                break;
            }
        }
        self.want_p(")")?;
        Ok(Expr::Call(last, args))
    }
    fn states(&mut self) -> R<Vec<String>> {
        let mut v = vec![self.ident()?];
        while self.eat_p("|") {
            v.push(self.ident()?);
        }
        Ok(v)
    }

    // --- statements ---
    fn block(&mut self) -> R<Vec<Stmt>> {
        self.want_p("{")?;
        self.nls();
        let mut out = vec![];
        while !self.eat_p("}") {
            if self.peek().is_none() {
                return self.err("}");
            }
            out.push(self.stmt()?);
            self.nls();
        }
        Ok(out)
    }
    fn class_items(&mut self) -> R<Vec<(u32, u32)>> {
        let mut v = vec![];
        loop {
            let lo = match self.peek() {
                Some(Tok::Str(s)) if s.len() == 1 => s[0] as u32,
                Some(Tok::Int(x)) => *x as u32,
                Some(Tok::UPlus(x)) => *x,
                _ => break,
            };
            self.i += 1;
            let hi = if self.eat_p("..") {
                match self.peek() {
                    Some(Tok::Str(s)) if s.len() == 1 => s[0] as u32,
                    Some(Tok::Int(x)) => *x as u32,
                    Some(Tok::UPlus(x)) => *x,
                    _ => return self.err("a range end"),
                }
            } else {
                self.i -= 1;
                lo
            };
            self.i += 1;
            v.push((lo, hi));
        }
        if v.is_empty() {
            return self.err("class items");
        }
        Ok(v)
    }
    fn words(&mut self) -> R<Vec<(Vec<u8>, Option<String>)>> {
        self.want_p("{")?;
        let mut v = vec![];
        loop {
            self.nls();
            if self.eat_p("}") {
                break;
            }
            let w = self.string()?;
            let alias = if self.eat_p("->") { Some(self.kind()?) } else { None };
            v.push((w, alias));
        }
        Ok(v)
    }
    fn stmt(&mut self) -> R<Stmt> {
        let kw = self.ident()?;
        let s = match kw.as_str() {
            "lexer" => Stmt::Lexer(self.ident()?),
            "class" => {
                let n = self.ident()?;
                self.want_p("=")?;
                Stmt::Class(n, self.class_items()?)
            }
            "mask" => {
                let n = self.ident()?;
                self.want_p("=")?;
                Stmt::Mask(n, self.expr()?)
            }
            "carry" | "next" => {
                let n = self.ident()?;
                self.want_p("=")?;
                let e = self.expr()?;
                if kw == "carry" {
                    Stmt::Carry(n, e)
                } else {
                    Stmt::Next(n, e)
                }
            }
            "scan" => {
                let name = self.ident()?;
                self.want_p("=")?;
                let f = self.ident()?;
                self.want_p("(")?;
                let s = if f == "reflect" {
                    let a = self.expr()?;
                    self.want_p("->")?;
                    let an = self.ident()?;
                    self.want_p(",")?;
                    let b = self.expr()?;
                    self.want_p("->")?;
                    let bn = self.ident()?;
                    self.want_p(")")?;
                    let reset = if self.eat_kw("reset") { Some(self.expr()?) } else { None };
                    let start_lane = if self.eat_kw("start") {
                        self.want_kw("lane")?;
                        true
                    } else {
                        false
                    };
                    Stmt::Reflect { name, a, an, b, bn, reset, start_lane }
                } else if f == "count" {
                    let up = self.expr()?;
                    self.want_p(",")?;
                    let down = self.expr()?;
                    self.want_p(")")?;
                    let (mut reset, mut max) = (None, None);
                    loop {
                        if self.eat_kw("reset") {
                            reset = Some(self.expr()?)
                        } else if self.eat_kw("max") {
                            max = Some(self.int()? as u32)
                        } else {
                            break;
                        }
                    }
                    let Some(max) = max else { return self.err("`max N`: a count's bound") };
                    Stmt::Count { name, up, down, reset, max }
                } else {
                    return self.err("reflect or count");
                };
                s
            }
            "block" => {
                let name = self.ident()?;
                if self.eat_p("=") {
                    let template = self.ident()?;
                    self.want_p("(")?;
                    let mut args = vec![self.expr()?];
                    while self.eat_p(",") {
                        args.push(self.expr()?)
                    }
                    self.want_p(")")?;
                    Stmt::Inst { name, template, args }
                } else {
                    self.want_p("(")?;
                    let mut params = vec![self.ident()?];
                    while self.eat_p(",") {
                        params.push(self.ident()?)
                    }
                    self.want_p(")")?;
                    Stmt::Block { name, params, body: self.block()? }
                }
            }
            "hypotheses" => {
                let name = self.ident()?;
                self.want_kw("split")?;
                let split = self.expr()?;
                self.want_kw("lanes")?;
                let mut lanes = vec![];
                while !self.is_p("{") {
                    lanes.push(self.ident()?)
                }
                Stmt::Hyp { name, split, lanes, body: self.block()? }
            }
            "rounds" => {
                let n = self.int()? as usize;
                let settle = self.eat_kw("until");
                if settle {
                    self.want_kw("settled")?;
                }
                Stmt::Rounds { n, settle, body: self.block()? }
            }
            "reject" => {
                let m = String::from_utf8_lossy(&self.string()?).into_owned();
                self.want_kw("when")?;
                Stmt::Reject(m, self.expr()?)
            }
            "exit" => Stmt::Exit(self.ident()?),
            "error" => {
                let m = String::from_utf8_lossy(&self.string()?).into_owned();
                self.want_kw("when")?;
                Stmt::Error(m, self.expr()?)
            }
            "skip" => Stmt::Skip(self.expr()?),
            "token" => {
                let kind = self.kind()?;
                self.want_p("=")?;
                self.want_kw("start")?;
                let start = self.expr()?;
                let through = if self.eat_kw("through") { Some(self.expr()?) } else { None };
                Stmt::Token { kind, start, through }
            }
            "munch" => {
                let kind = self.ident()?;
                self.want_kw("over")?;
                let over = self.expr()?;
                Stmt::Munch { kind, over, lits: self.words()? }
            }
            "keywords" => {
                let kind = self.ident()?;
                let words = self.words()?;
                let soft = if self.eat_kw("soft") { self.words()?.into_iter().map(|w| w.0).collect() } else { vec![] };
                let required = self.eat_kw("required");
                Stmt::Keywords { kind, words, soft, required }
            }
            "ucode" => {
                self.want_kw("over")?;
                let over = self.expr()?;
                self.want_p("{")?;
                let mut classes = vec![];
                loop {
                    self.nls();
                    if self.eat_p("}") {
                        break;
                    }
                    self.want_kw("class")?;
                    let n = self.ident()?;
                    self.want_p("=")?;
                    if self.eat_kw("table") {
                        let f = String::from_utf8_lossy(&self.string()?).into_owned();
                        classes.push((n, UClass::Table(f, self.ident()?)));
                    } else {
                        classes.push((n, UClass::Points(self.class_items()?)));
                    }
                }
                Stmt::Ucode { over, classes }
            }
            "machine" => {
                let name = self.ident()?;
                Stmt::Machine(name, self.machine()?)
            }
            _ => return self.err("a statement"),
        };
        self.end_stmt()?;
        Ok(s)
    }
    fn machine(&mut self) -> R<MachineDef> {
        let mut m = MachineDef::default();
        self.want_p("{")?;
        loop {
            self.nls();
            if self.eat_p("}") {
                break;
            }
            let w = self.ident()?;
            match w.as_str() {
                "events" => {
                    self.want_p("{")?;
                    loop {
                        self.nls();
                        if self.eat_p("}") {
                            break;
                        }
                        let s = self.ident()?;
                        self.want_p("=")?;
                        m.events.push((s, self.expr()?));
                    }
                }
                "states" => {
                    while let Some(Tok::Ident(_)) = self.peek() {
                        m.states.push(self.ident()?)
                    }
                }
                "group" => {
                    let g = self.ident()?;
                    self.want_p("=")?;
                    let mut v = vec![];
                    while let Some(Tok::Ident(_)) = self.peek() {
                        v.push(self.ident()?)
                    }
                    m.groups.push((g, v));
                }
                "start" => m.start = self.ident()?,
                "final" => {
                    while let Some(Tok::Ident(_)) = self.peek() {
                        m.finals.push(self.ident()?)
                    }
                }
                st => loop {
                    self.want_kw("on")?;
                    let mut syms = vec![self.ident()?];
                    while self.eat_p("|") {
                        syms.push(self.ident()?)
                    }
                    self.want_p("->")?;
                    m.rules.push((st.to_string(), syms, self.ident()?));
                    if !self.eat_p(",") {
                        break;
                    }
                },
            }
            self.end_stmt()?;
        }
        if m.finals.is_empty() {
            m.finals.push(m.start.clone())
        }
        Ok(m)
    }
}

pub fn parse(src: &str) -> Result<Vec<Stmt>, String> {
    let mut p = P { t: lex(src)?, i: 0 };
    p.nls();
    let mut out = vec![];
    while p.peek().is_some() {
        out.push(p.stmt()?);
        p.nls();
    }
    Ok(out)
}
