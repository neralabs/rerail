//! A recursive-descent parser for the SQLite SELECT subset the batch-6 policies use.
use super::value::V;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Quoted(String),
    Str(String),
    Blob(String),
    Int(i64),
    Real(f64),
    Sym(&'static str),
}

pub fn lex(src: &str) -> Result<Vec<(Tok, usize, usize)>, String> {
    let b = src.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'-' && b.get(i + 1) == Some(&b'-') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        let start = i;
        if c == b'\'' {
            let mut s = String::new();
            i += 1;
            loop {
                if i >= b.len() {
                    return Err("unterminated string".into());
                }
                if b[i] == b'\'' {
                    if b.get(i + 1) == Some(&b'\'') {
                        s.push('\'');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                let ch = src[i..].chars().next().unwrap();
                s.push(ch);
                i += ch.len_utf8();
            }
            out.push((Tok::Str(s), start, i));
            continue;
        }
        if c == b'"' {
            let mut s = String::new();
            i += 1;
            loop {
                if i >= b.len() {
                    return Err("unterminated identifier".into());
                }
                if b[i] == b'"' {
                    if b.get(i + 1) == Some(&b'"') {
                        s.push('"');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                let ch = src[i..].chars().next().unwrap();
                s.push(ch);
                i += ch.len_utf8();
            }
            out.push((Tok::Quoted(s), start, i));
            continue;
        }
        if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            let mut real = false;
            if i < b.len() && b[i] == b'.' {
                real = true;
                i += 1;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                real = true;
                i += 1;
                if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
                    i += 1;
                }
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let t = &src[start..i];
            out.push((
                if real {
                    Tok::Real(t.parse().map_err(|_| "bad number")?)
                } else {
                    match t.parse::<i64>() {
                        Ok(v) => Tok::Int(v),
                        Err(_) => Tok::Real(t.parse().map_err(|_| "bad number")?),
                    }
                },
                start,
                i,
            ));
            continue;
        }
        if (c == b'x' || c == b'X') && b.get(i + 1) == Some(&b'\'') {
            // A blob literal; the policies only read one as `CAST(X'..' AS TEXT)`, so its bytes
            // become the UTF-8 text the cast produces.
            let close = src[i + 2..].find('\'').ok_or("unterminated blob")? + i + 2;
            let hex = &src[i + 2..close];
            if hex.len() % 2 != 0 {
                return Err("odd blob literal".into());
            }
            let bytes = (0..hex.len()).step_by(2).map(|k| u8::from_str_radix(&hex[k..k + 2], 16)).collect::<Result<Vec<_>, _>>().map_err(|_| "bad blob literal")?;
            let text = String::from_utf8(bytes).map_err(|_| "blob literal is not UTF-8 text")?;
            out.push((Tok::Blob(text), start, close + 1));
            i = close + 1;
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$') {
                i += 1;
            }
            out.push((Tok::Ident(src[start..i].to_owned()), start, i));
            continue;
        }
        let two = &src[i..(i + 2).min(src.len())];
        let sym2 = ["||", "<=", ">=", "<>", "!=", "=="].iter().find(|s| two == **s);
        if let Some(s) = sym2 {
            out.push((Tok::Sym(s), start, i + 2));
            i += 2;
            continue;
        }
        let sym1 = ["(", ")", ",", ".", "*", "+", "-", "/", "%", "=", "<", ">", ";"]
            .iter()
            .find(|s| s.as_bytes()[0] == c)
            .ok_or_else(|| format!("unexpected character `{}` at {i}", c as char))?;
        out.push((Tok::Sym(sym1), start, i + 1));
        i += 1;
    }
    Ok(out)
}

#[derive(Clone, Debug)]
pub enum Expr {
    Lit(V),
    /// `X'..'` decoded as UTF-8 text (only valid as `CAST(X'..' AS TEXT)`).
    Blob(String),
    Col { table: Option<String>, name: String },
    Unary(&'static str, Box<Expr>),
    Bin(&'static str, Box<Expr>, Box<Expr>),
    Func { name: String, args: Vec<Expr>, star: bool, distinct: bool, over: Option<Window> },
    Case { operand: Option<Box<Expr>>, whens: Vec<(Expr, Expr)>, else_: Option<Box<Expr>> },
    Cast(Box<Expr>, String),
    InList { e: Box<Expr>, list: Vec<Expr>, not: bool },
    InSub { e: Box<Expr>, q: Box<Query>, not: bool },
    Exists { q: Box<Query>, not: bool },
    Sub(Box<Query>),
    IsNull(Box<Expr>, bool),
    Is(Box<Expr>, Box<Expr>, bool),
    Like(Box<Expr>, Box<Expr>, bool),
    Between(Box<Expr>, Box<Expr>, Box<Expr>, bool),
}

#[derive(Clone, Debug)]
pub struct Window {
    pub partition: Vec<Expr>,
    pub order: Vec<(Expr, bool)>,
}

#[derive(Clone, Debug)]
pub enum SelItem {
    Star,
    TableStar(String),
    Expr { e: Expr, alias: Option<String>, text: String },
}

#[derive(Clone, Debug)]
pub enum Source {
    Table(String),
    Func(String, Vec<Expr>),
    Sub(Box<Query>),
}

#[derive(Clone, Debug, PartialEq, Copy)]
pub enum JoinKind {
    Inner,
    Left,
}

#[derive(Clone, Debug)]
pub struct FromItem {
    pub source: Source,
    pub alias: Option<String>,
    pub kind: JoinKind,
    pub on: Option<Expr>,
}

#[derive(Clone, Debug)]
pub struct Select {
    pub distinct: bool,
    pub items: Vec<SelItem>,
    pub from: Vec<FromItem>,
    pub where_: Option<Expr>,
    pub group_by: Vec<Expr>,
    pub having: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq, Copy)]
pub enum SetOp {
    Union,
    UnionAll,
}

#[derive(Clone, Debug)]
pub enum Body {
    Select(Box<Select>),
    Compound(Box<Body>, SetOp, Box<Body>),
}

#[derive(Clone, Debug)]
pub struct Cte {
    pub name: String,
    pub cols: Option<Vec<String>>,
    pub q: Query,
}

#[derive(Clone, Debug)]
pub struct Query {
    pub recursive: bool,
    pub ctes: Vec<Cte>,
    pub body: Body,
    pub order: Vec<(Expr, bool)>,
    pub limit: Option<Expr>,
    pub offset: Option<Expr>,
}

pub struct Parser<'s> {
    src: &'s str,
    toks: Vec<(Tok, usize, usize)>,
    pos: usize,
}

const RESERVED: &[&str] = &[
    "select", "from", "where", "group", "having", "order", "limit", "offset", "union", "join",
    "left", "inner", "cross", "on", "as", "and", "or", "not", "in", "is", "null", "case", "when",
    "then", "else", "end", "exists", "distinct", "all", "by", "with", "recursive", "like",
    "between", "cast", "over", "partition", "outer", "natural", "using", "except", "intersect",
    "asc", "desc", "materialized", "glob", "escape", "collate",
];

pub fn parse_query(src: &str) -> Result<Query, String> {
    let mut p = Parser { src, toks: lex(src)?, pos: 0 };
    let q = p.query()?;
    while p.peek_sym(";") {
        p.pos += 1;
    }
    if p.pos != p.toks.len() {
        return Err(format!("unexpected trailing token {:?}", p.toks[p.pos].0));
    }
    Ok(q)
}

pub fn parse_expr(src: &str) -> Result<Expr, String> {
    let mut p = Parser { src, toks: lex(src)?, pos: 0 };
    let e = p.expr()?;
    if p.pos != p.toks.len() {
        return Err("unexpected trailing token in expression".into());
    }
    Ok(e)
}

impl<'s> Parser<'s> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.0)
    }
    fn kw(&self, k: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(s)) if s.eq_ignore_ascii_case(k))
    }
    fn kw_at(&self, off: usize, k: &str) -> bool {
        matches!(self.toks.get(self.pos + off).map(|t| &t.0), Some(Tok::Ident(s)) if s.eq_ignore_ascii_case(k))
    }
    fn eat_kw(&mut self, k: &str) -> bool {
        if self.kw(k) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect_kw(&mut self, k: &str) -> Result<(), String> {
        if self.eat_kw(k) { Ok(()) } else { Err(format!("expected {k}, found {:?}", self.peek())) }
    }
    fn peek_sym(&self, s: &str) -> bool {
        matches!(self.peek(), Some(Tok::Sym(x)) if *x == s)
    }
    fn eat_sym(&mut self, s: &str) -> bool {
        if self.peek_sym(s) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect_sym(&mut self, s: &str) -> Result<(), String> {
        if self.eat_sym(s) { Ok(()) } else { Err(format!("expected `{s}`, found {:?} at token {}", self.peek(), self.pos)) }
    }
    fn ident(&mut self) -> Result<String, String> {
        match self.peek().cloned() {
            Some(Tok::Ident(s)) => {
                self.pos += 1;
                Ok(s)
            }
            Some(Tok::Quoted(s)) => {
                self.pos += 1;
                Ok(s)
            }
            t => Err(format!("expected identifier, found {t:?}")),
        }
    }
    fn is_name_token(&self) -> bool {
        match self.peek() {
            Some(Tok::Quoted(_)) => true,
            Some(Tok::Ident(s)) => !RESERVED.iter().any(|r| s.eq_ignore_ascii_case(r)),
            _ => false,
        }
    }

    pub fn query(&mut self) -> Result<Query, String> {
        let mut recursive = false;
        let mut ctes = Vec::new();
        if self.eat_kw("with") {
            recursive = self.eat_kw("recursive");
            loop {
                let name = self.ident()?;
                let cols = if self.eat_sym("(") {
                    let mut cols = vec![self.ident()?];
                    while self.eat_sym(",") {
                        cols.push(self.ident()?);
                    }
                    self.expect_sym(")")?;
                    Some(cols)
                } else {
                    None
                };
                self.expect_kw("as")?;
                if self.eat_kw("not") {
                    self.expect_kw("materialized")?;
                } else {
                    self.eat_kw("materialized");
                }
                self.expect_sym("(")?;
                let q = self.query()?;
                self.expect_sym(")")?;
                ctes.push(Cte { name, cols, q });
                if !self.eat_sym(",") {
                    break;
                }
            }
        }
        let mut body = self.select_core()?;
        loop {
            if self.kw("union") {
                self.pos += 1;
                let op = if self.eat_kw("all") { SetOp::UnionAll } else { SetOp::Union };
                let right = self.select_core()?;
                body = Body::Compound(Box::new(body), op, Box::new(right));
            } else if self.kw("except") || self.kw("intersect") {
                return Err("EXCEPT/INTERSECT unsupported".into());
            } else {
                break;
            }
        }
        let mut order = Vec::new();
        if self.kw("order") && self.kw_at(1, "by") {
            self.pos += 2;
            order = self.order_list()?;
        }
        let mut limit = None;
        let mut offset = None;
        if self.eat_kw("limit") {
            limit = Some(self.expr()?);
            if self.eat_kw("offset") {
                offset = Some(self.expr()?);
            } else if self.eat_sym(",") {
                offset = limit.take();
                limit = Some(self.expr()?);
            }
        }
        Ok(Query { recursive, ctes, body, order, limit, offset })
    }

    fn order_list(&mut self) -> Result<Vec<(Expr, bool)>, String> {
        let mut v = Vec::new();
        loop {
            let e = self.expr()?;
            if self.eat_kw("collate") {
                let c = self.ident()?;
                if !c.eq_ignore_ascii_case("binary") {
                    return Err(format!("collation {c}"));
                }
            }
            let desc = if self.eat_kw("desc") {
                true
            } else {
                self.eat_kw("asc");
                false
            };
            v.push((e, desc));
            if !self.eat_sym(",") {
                break;
            }
        }
        Ok(v)
    }

    fn select_core(&mut self) -> Result<Body, String> {
        if self.peek_sym("(") {
            return Err("parenthesised compound arm unsupported".into());
        }
        self.expect_kw("select")?;
        let distinct = if self.eat_kw("distinct") {
            true
        } else {
            self.eat_kw("all");
            false
        };
        let mut items = Vec::new();
        loop {
            if self.eat_sym("*") {
                items.push(SelItem::Star);
            } else if matches!(self.peek(), Some(Tok::Ident(_) | Tok::Quoted(_)))
                && matches!(self.toks.get(self.pos + 1).map(|t| &t.0), Some(Tok::Sym(".")))
                && matches!(self.toks.get(self.pos + 2).map(|t| &t.0), Some(Tok::Sym("*")))
            {
                let t = self.ident()?;
                self.pos += 2;
                items.push(SelItem::TableStar(t));
            } else {
                let start = self.toks[self.pos].1;
                let e = self.expr()?;
                let end = self.toks[self.pos - 1].2;
                let alias = if self.eat_kw("as") {
                    Some(self.ident()?)
                } else if self.is_name_token() {
                    Some(self.ident()?)
                } else {
                    None
                };
                items.push(SelItem::Expr { e, alias, text: self.src[start..end].to_owned() });
            }
            if !self.eat_sym(",") {
                break;
            }
        }
        let mut from = Vec::new();
        if self.eat_kw("from") {
            from.push(self.from_item(JoinKind::Inner)?);
            loop {
                if self.eat_sym(",") {
                    from.push(self.from_item(JoinKind::Inner)?);
                    continue;
                }
                let kind = if self.kw("left") {
                    self.pos += 1;
                    self.eat_kw("outer");
                    self.expect_kw("join")?;
                    JoinKind::Left
                } else if self.kw("inner") || self.kw("cross") {
                    self.pos += 1;
                    self.expect_kw("join")?;
                    JoinKind::Inner
                } else if self.eat_kw("join") {
                    JoinKind::Inner
                } else {
                    break;
                };
                let mut item = self.from_item(kind)?;
                if self.eat_kw("on") {
                    item.on = Some(self.expr()?);
                } else if self.kw("using") {
                    return Err("USING unsupported".into());
                }
                from.push(item);
            }
        }
        let where_ = if self.eat_kw("where") { Some(self.expr()?) } else { None };
        let mut group_by = Vec::new();
        let mut having = None;
        if self.kw("group") && self.kw_at(1, "by") {
            self.pos += 2;
            loop {
                group_by.push(self.expr()?);
                if !self.eat_sym(",") {
                    break;
                }
            }
        }
        // HAVING with or without GROUP BY (without: an aggregate select, one group; SQLite 3.39+).
        if self.eat_kw("having") {
            having = Some(self.expr()?);
        }
        Ok(Body::Select(Box::new(Select { distinct, items, from, where_, group_by, having })))
    }

    fn from_item(&mut self, kind: JoinKind) -> Result<FromItem, String> {
        let source = if self.eat_sym("(") {
            let q = self.query()?;
            self.expect_sym(")")?;
            Source::Sub(Box::new(q))
        } else {
            let name = self.ident()?;
            if self.eat_sym("(") {
                let mut args = Vec::new();
                if !self.peek_sym(")") {
                    loop {
                        args.push(self.expr()?);
                        if !self.eat_sym(",") {
                            break;
                        }
                    }
                }
                self.expect_sym(")")?;
                Source::Func(name, args)
            } else {
                Source::Table(name)
            }
        };
        let alias = if self.eat_kw("as") {
            Some(self.ident()?)
        } else if self.is_name_token() {
            Some(self.ident()?)
        } else {
            None
        };
        Ok(FromItem { source, alias, kind, on: None })
    }

    pub fn expr(&mut self) -> Result<Expr, String> {
        self.or()
    }
    fn or(&mut self) -> Result<Expr, String> {
        let mut l = self.and()?;
        while self.eat_kw("or") {
            let r = self.and()?;
            l = Expr::Bin("or", Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn and(&mut self) -> Result<Expr, String> {
        let mut l = self.not()?;
        while self.eat_kw("and") {
            let r = self.not()?;
            l = Expr::Bin("and", Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn not(&mut self) -> Result<Expr, String> {
        if self.kw("not") && !self.kw_at(1, "exists") {
            self.pos += 1;
            let e = self.not()?;
            return Ok(Expr::Unary("not", Box::new(e)));
        }
        self.equality()
    }
    fn equality(&mut self) -> Result<Expr, String> {
        let mut l = self.comparison()?;
        loop {
            if self.eat_sym("=") || self.eat_sym("==") {
                let r = self.comparison()?;
                l = Expr::Bin("=", Box::new(l), Box::new(r));
            } else if self.eat_sym("!=") || self.eat_sym("<>") {
                let r = self.comparison()?;
                l = Expr::Bin("!=", Box::new(l), Box::new(r));
            } else if self.kw("is") {
                self.pos += 1;
                let not = self.eat_kw("not");
                if self.eat_kw("null") {
                    l = Expr::IsNull(Box::new(l), not);
                } else {
                    let r = self.comparison()?;
                    l = Expr::Is(Box::new(l), Box::new(r), not);
                }
            } else if self.kw("not") && (self.kw_at(1, "in") || self.kw_at(1, "like") || self.kw_at(1, "between")) {
                self.pos += 1;
                l = self.in_like(l, true)?;
            } else if self.kw("in") || self.kw("like") || self.kw("between") {
                l = self.in_like(l, false)?;
            } else if self.kw("isnull") {
                self.pos += 1;
                l = Expr::IsNull(Box::new(l), false);
            } else if self.kw("notnull") {
                self.pos += 1;
                l = Expr::IsNull(Box::new(l), true);
            } else {
                break;
            }
        }
        Ok(l)
    }
    fn in_like(&mut self, l: Expr, not: bool) -> Result<Expr, String> {
        if self.eat_kw("in") {
            self.expect_sym("(")?;
            if self.kw("select") || self.kw("with") {
                let q = self.query()?;
                self.expect_sym(")")?;
                return Ok(Expr::InSub { e: Box::new(l), q: Box::new(q), not });
            }
            let mut list = Vec::new();
            if !self.peek_sym(")") {
                loop {
                    list.push(self.expr()?);
                    if !self.eat_sym(",") {
                        break;
                    }
                }
            }
            self.expect_sym(")")?;
            return Ok(Expr::InList { e: Box::new(l), list, not });
        }
        if self.eat_kw("like") {
            let r = self.comparison()?;
            if self.kw("escape") {
                return Err("LIKE ESCAPE unsupported".into());
            }
            return Ok(Expr::Like(Box::new(l), Box::new(r), not));
        }
        self.expect_kw("between")?;
        let lo = self.comparison()?;
        self.expect_kw("and")?;
        let hi = self.comparison()?;
        Ok(Expr::Between(Box::new(l), Box::new(lo), Box::new(hi), not))
    }
    fn comparison(&mut self) -> Result<Expr, String> {
        let mut l = self.additive()?;
        loop {
            let op = ["<=", ">=", "<", ">"].iter().find(|s| self.peek_sym(s)).copied();
            match op {
                Some(op) => {
                    self.pos += 1;
                    let r = self.additive()?;
                    l = Expr::Bin(op, Box::new(l), Box::new(r));
                }
                None => break,
            }
        }
        Ok(l)
    }
    fn additive(&mut self) -> Result<Expr, String> {
        let mut l = self.multiplicative()?;
        loop {
            let op = ["+", "-"].iter().find(|s| self.peek_sym(s)).copied();
            match op {
                Some(op) => {
                    self.pos += 1;
                    let r = self.multiplicative()?;
                    l = Expr::Bin(op, Box::new(l), Box::new(r));
                }
                None => break,
            }
        }
        Ok(l)
    }
    fn multiplicative(&mut self) -> Result<Expr, String> {
        let mut l = self.concat()?;
        loop {
            let op = ["*", "/", "%"].iter().find(|s| self.peek_sym(s)).copied();
            match op {
                Some(op) => {
                    self.pos += 1;
                    let r = self.concat()?;
                    l = Expr::Bin(op, Box::new(l), Box::new(r));
                }
                None => break,
            }
        }
        Ok(l)
    }
    fn concat(&mut self) -> Result<Expr, String> {
        let mut l = self.unary()?;
        while self.eat_sym("||") {
            let r = self.unary()?;
            l = Expr::Bin("||", Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat_sym("-") {
            let e = self.unary()?;
            return Ok(match e {
                Expr::Lit(V::Int(i)) => Expr::Lit(V::Int(-i)),
                Expr::Lit(V::Real(f)) => Expr::Lit(V::Real(-f)),
                e => Expr::Unary("-", Box::new(e)),
            });
        }
        if self.eat_sym("+") {
            let e = self.unary()?;
            return Ok(Expr::Unary("+", Box::new(e)));
        }
        let e = self.primary()?;
        if self.kw("collate") {
            self.pos += 1;
            let c = self.ident()?;
            if !c.eq_ignore_ascii_case("binary") {
                return Err(format!("collation {c}"));
            }
        }
        Ok(e)
    }
    fn primary(&mut self) -> Result<Expr, String> {
        let tok = self.peek().cloned().ok_or("unexpected end of SQL")?;
        match tok {
            Tok::Blob(s) => {
                self.pos += 1;
                Ok(Expr::Blob(s))
            }
            Tok::Str(s) => {
                self.pos += 1;
                Ok(Expr::Lit(V::text(&s)))
            }
            Tok::Int(i) => {
                self.pos += 1;
                Ok(Expr::Lit(V::Int(i)))
            }
            Tok::Real(f) => {
                self.pos += 1;
                Ok(Expr::Lit(V::Real(f)))
            }
            Tok::Sym("(") => {
                self.pos += 1;
                if self.kw("select") || self.kw("with") {
                    let q = self.query()?;
                    self.expect_sym(")")?;
                    return Ok(Expr::Sub(Box::new(q)));
                }
                let e = self.expr()?;
                if self.peek_sym(",") {
                    return Err("row values unsupported".into());
                }
                self.expect_sym(")")?;
                Ok(e)
            }
            Tok::Ident(ref s) if s.eq_ignore_ascii_case("null") => {
                self.pos += 1;
                Ok(Expr::Lit(V::Null))
            }
            Tok::Ident(ref s) if s.eq_ignore_ascii_case("true") => {
                self.pos += 1;
                Ok(Expr::Lit(V::Int(1)))
            }
            Tok::Ident(ref s) if s.eq_ignore_ascii_case("false") => {
                self.pos += 1;
                Ok(Expr::Lit(V::Int(0)))
            }
            Tok::Ident(ref s) if s.eq_ignore_ascii_case("not") && self.kw_at(1, "exists") => {
                self.pos += 2;
                self.expect_sym("(")?;
                let q = self.query()?;
                self.expect_sym(")")?;
                Ok(Expr::Exists { q: Box::new(q), not: true })
            }
            Tok::Ident(ref s) if s.eq_ignore_ascii_case("exists") => {
                self.pos += 1;
                self.expect_sym("(")?;
                let q = self.query()?;
                self.expect_sym(")")?;
                Ok(Expr::Exists { q: Box::new(q), not: false })
            }
            Tok::Ident(ref s) if s.eq_ignore_ascii_case("case") => {
                self.pos += 1;
                let operand = if self.kw("when") { None } else { Some(Box::new(self.expr()?)) };
                let mut whens = Vec::new();
                while self.eat_kw("when") {
                    let w = self.expr()?;
                    self.expect_kw("then")?;
                    let t = self.expr()?;
                    whens.push((w, t));
                }
                let else_ = if self.eat_kw("else") { Some(Box::new(self.expr()?)) } else { None };
                self.expect_kw("end")?;
                Ok(Expr::Case { operand, whens, else_ })
            }
            Tok::Ident(ref s) if s.eq_ignore_ascii_case("cast") => {
                self.pos += 1;
                self.expect_sym("(")?;
                let e = self.expr()?;
                self.expect_kw("as")?;
                let mut ty = self.ident()?;
                while matches!(self.peek(), Some(Tok::Ident(_))) {
                    ty.push(' ');
                    ty.push_str(&self.ident()?);
                }
                self.expect_sym(")")?;
                Ok(Expr::Cast(Box::new(e), ty))
            }
            Tok::Ident(_) | Tok::Quoted(_) => {
                let name = self.ident()?;
                if self.peek_sym("(") && matches!(tok, Tok::Ident(_)) {
                    self.pos += 1;
                    let mut args = Vec::new();
                    let mut star = false;
                    let distinct = self.eat_kw("distinct");
                    if self.eat_sym("*") {
                        star = true;
                    } else if !self.peek_sym(")") {
                        loop {
                            args.push(self.expr()?);
                            if !self.eat_sym(",") {
                                break;
                            }
                        }
                    }
                    self.expect_sym(")")?;
                    let over = if self.eat_kw("over") {
                        self.expect_sym("(")?;
                        let mut partition = Vec::new();
                        if self.kw("partition") {
                            self.pos += 1;
                            self.expect_kw("by")?;
                            loop {
                                partition.push(self.expr()?);
                                if !self.eat_sym(",") {
                                    break;
                                }
                            }
                        }
                        let mut order = Vec::new();
                        if self.kw("order") {
                            self.pos += 1;
                            self.expect_kw("by")?;
                            order = self.order_list()?;
                        }
                        self.expect_sym(")")?;
                        Some(Window { partition, order })
                    } else {
                        None
                    };
                    return Ok(Expr::Func { name: name.to_ascii_lowercase(), args, star, distinct, over });
                }
                if self.eat_sym(".") {
                    let col = self.ident()?;
                    return Ok(Expr::Col { table: Some(name), name: col });
                }
                Ok(Expr::Col { table: None, name })
            }
            t => Err(format!("unexpected token {t:?}")),
        }
    }
}
