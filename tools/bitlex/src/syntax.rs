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

/// A token mark's choice (`Stmt::TokenMark`): a token of a kind, or a token whose text is a word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenElem {
    Kind(String),
    Word(Vec<u8>),
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

/// A number's definition (`number N = …`): each byte's value, an unsigned 32-bit number kept
/// mod 2^32 (docs/parser-generators/03-bitlex.md §2.8).
#[derive(Clone, Debug)]
pub enum NExpr {
    /// The byte's place in the file, from 0.
    Position,
    /// Another number.
    Name(String),
    /// `fill(s, e, v)` (`back: false`) or `back_fill(s, e, v)`: on the bytes of the mask's fill,
    /// `v` at the byte of `s` the fill came from (the nearest, as each byte of `s` starts a fill
    /// of its own); 0 elsewhere.
    Fill { s: Expr, e: Expr, v: Box<NExpr>, back: bool },
    /// `a - b`, mod 2^32.
    Sub(Box<NExpr>, Box<NExpr>),
}

/// `regions H { K = O .. C … }`: kinds of region that hide each other, as strings, comments
/// and quoted literals do. A region opens at a byte of its opener `O` not hidden, and its fill
/// (`f_K`) runs from there up to, not including, the next byte of its closer `C`. The bytes it
/// hides (`H`) run from the byte after its opener through its closer's last byte. Taken left
/// to right: the regions' fixed point, `rounds until settled` (`body`).
#[derive(Clone, Debug)]
pub struct Regions {
    /// The hidden bytes' name: the carry.
    pub hidden: String,
    pub kinds: Vec<Region>,
}

/// A kind of region (`Regions`).
#[derive(Clone, Debug)]
pub struct Region {
    /// Its name `K`: the openers taken are `o_K`, the fill `f_K`.
    pub kind: String,
    pub opener: Expr,
    pub closer: Expr,
    /// `bytes N`: the closer's bytes, from its first (1 to 3).
    pub bytes: u8,
    /// `outside`: the closer is not the region's (a line comment's newline). One byte.
    pub outside: bool,
    /// `else E bytes M`: an opener with no byte of the closer after it anywhere in the file
    /// opens no region; it is `E`, `M` bytes from the opener's first, hidden after it.
    pub empty: Option<(String, u8)>,
}

impl Regions {
    /// The `rounds until settled` body the regions mean:
    ///
    /// ```text
    /// mask K_later = next(back_fill(C, empty))       # a kind with `else`
    /// carry H      = empty
    /// mask o_K     = O & ~H [& K_later]
    /// mask E       = O & ~H & ~K_later                # with `else E`
    /// mask f_K     = fill(o_K, C)
    /// next H       = prev(f_K) | … | prev(f_K, N)     # `outside`: prev(f_K) & ~C
    ///              | prev(E) | … | prev(E, M - 1)
    /// ```
    pub fn body(&self) -> Vec<Stmt> {
        let name = |s: &str| Expr::Name(s.to_string());
        let and = |a: Expr, b: Expr| Expr::And(Box::new(a), Box::new(b));
        let not = |a: Expr| Expr::Not(Box::new(a));
        let call = |f: &str, args: Vec<Arg>| Expr::Call(f.to_string(), args);
        let prev = |e: &str, k: u8| if k == 1 { call("prev", vec![Arg::E(name(e))]) } else { call("prev", vec![Arg::E(name(e)), Arg::N(u64::from(k))]) };
        let mut out = Vec::new();
        for r in self.kinds.iter().filter(|r| r.empty.is_some()) {
            let later = call("next", vec![Arg::E(call("back_fill", vec![Arg::E(r.closer.clone()), Arg::E(Expr::Empty)]))]);
            out.push(Stmt::Mask(r.later(), later));
        }
        out.push(Stmt::Carry(self.hidden.clone(), Expr::Empty));
        let mut hide: Option<Expr> = None;
        let mut or = |e: Expr| hide = Some(match hide.take() {
            Some(h) => Expr::Or(Box::new(h), Box::new(e)),
            None => e,
        });
        for r in &self.kinds {
            let open = and(r.opener.clone(), not(name(&self.hidden)));
            let o = format!("o_{}", r.kind);
            let f = format!("f_{}", r.kind);
            match &r.empty {
                Some((e, m)) => {
                    out.push(Stmt::Mask(o.clone(), and(open.clone(), name(&r.later()))));
                    out.push(Stmt::Mask(e.clone(), and(open, not(name(&r.later())))));
                    for k in 1..*m {
                        or(prev(e, k));
                    }
                }
                None => out.push(Stmt::Mask(o.clone(), open)),
            }
            out.push(Stmt::Mask(f.clone(), call("fill", vec![Arg::E(name(&o)), Arg::E(r.closer.clone())])));
            if r.outside {
                or(and(prev(&f, 1), not(r.closer.clone())));
            } else {
                for k in 1..=r.bytes {
                    or(prev(&f, k));
                }
            }
        }
        out.push(Stmt::Next(self.hidden.clone(), hide.unwrap_or(Expr::Empty)));
        out
    }
}

impl Region {
    /// The name of its `else`'s mask: whether a byte of the closer comes after a byte.
    pub fn later(&self) -> String {
        format!("{}_later", self.kind)
    }
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

/// A tree rule's content (docs/parser-generators/18-tree-as-grammar.md §1): what a node spans,
/// as tree-sitter's `grammar.js` states it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gram {
    /// A quoted token: a keyword's kind, `'{'`.
    Lit(String),
    /// A token kind, a node or a choice, by name.
    Sym(String),
    Seq(Vec<Gram>),
    Alt(Vec<Gram>),
    /// `[A]`.
    Opt(Box<Gram>),
    /// `A*`.
    Star(Box<Gram>),
    /// `A+`.
    Plus(Box<Gram>),
    /// `f: A`: the node `A` gives has field `f`.
    Field(String, Box<Gram>),
    /// `retag K A`: on entering `A`, the innermost open node becomes a `K` (its kind decided by
    /// the token `A` starts with).
    Retag(String, Box<Gram>),
    /// `merge 'K' A`, `A` a token: its leaf and the leaf just before it become one leaf of kind
    /// `K`, from the first's start to its end (`.` and `decl`, `.decl`).
    Merge(String, Box<Gram>),
    /// `wrap K F [as M] REST…`: a node `K` around the node just made at this level (the last
    /// child of the innermost open node) and what `REST` matches. `K` takes the moved node's
    /// field; the moved node takes field `F` (`_`: none) and, with `as M`, kind `M`.
    Wrap { kind: String, field: Option<String>, moved: Option<String>, body: Box<Gram> },
    /// `balanced K OPEN CLOSE`: one leaf of kind `K` from an `OPEN` token through its matching
    /// `CLOSE` (pairs inside counted), every token between part of its text.
    Balanced { kind: String, open: String, close: String },
    /// `any [except 'A' …]`: a token of any kind the grammar may see but those listed (marks
    /// and extras aside); with `merge K` or `as K`, a raw text's.
    Any(Vec<String>),
    /// `relabel F [as M] [inner G] REST`: the node just made at this level takes field `F`
    /// (`_`: keeps its own) and kind `M`, and the node its wrap holds first takes field `G`;
    /// `REST` follows; no node around it.
    Relabel { field: Option<String>, moved: Option<String>, inner: Option<String>, body: Box<Gram> },
    /// `fold K REST`: at the token `REST` starts with, the node open at this level holds two
    /// leaves and nothing else; they become one leaf of kind `K`, with the first one's field.
    Fold { kind: String, body: Box<Gram> },
    /// `A as K`, `A` a token: its leaf has kind `K` (`identifier as variable`).
    As(String, Box<Gram>),
    /// `reread R REST`: at the token `REST` starts with, the node just made at this level and
    /// every node inside it are read by reading `R` (a `reading` statement), as the tree of
    /// another rule: a cover grammar's node, once the token after it says which it is.
    Reread { reading: String, body: Box<Gram> },
    /// `with F A`, `without F A`, `A` a nested rule: its level, and the levels it calls, run
    /// with flag `F` set (`with`) or clear; the caller's flags are its own again after it.
    Flag { flag: String, set: bool, body: Box<Gram> },
    /// `unless F A`: `A` where flag `F` is clear; where it is set, as if `A` were not there.
    Unless { flag: String, body: Box<Gram> },
}

/// What a `reading` does with a node of a kind it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadKind {
    /// `K` or `K as M`: the node stays, of kind `M` (its own, for `K`); the nodes inside it are
    /// read too.
    As(String),
    /// `K gone`: the node goes, and the nodes inside it take its place; they are read too.
    Gone,
    /// `keep K`: the node stays as it is, with every node inside it, unread.
    Keep,
}

/// `reading NAME = ITEM, …`: how `reread` reads a node and the nodes inside it. A node of a
/// kind no item names refuses the file (an extra's leaf aside, which stays as it is). A node
/// read, not the first, with a field that `field F as G` names takes field `G`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Reading {
    pub kinds: Vec<(String, ReadKind)>,
    pub fields: Vec<(String, String)>,
    /// `root gone`, `root as M`: what becomes of the node read itself, whatever its kind; the
    /// kinds then name only the nodes inside it.
    pub root: Option<ReadKind>,
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
    /// `number N = …`: a value per byte (`NExpr`).
    Number(String, NExpr),
    Machine(String, MachineDef),
    Block { name: String, params: Vec<String>, body: Vec<Stmt> },
    Inst { name: String, template: String, args: Vec<Expr> },
    Hyp { name: String, split: Expr, lanes: Vec<String>, body: Vec<Stmt> },
    Reject(String, Expr),
    Exit(String),
    /// At most `n` rounds. `until settled`: a backend may stop once a round changes no carry;
    /// without it, every round runs. Either way the values are the same. With no `n`
    /// (`rounds until settled`), the fixed point itself: the body must be strictly causal
    /// (`strictly_causal`), so it always settles, a byte more each round at least.
    ///
    /// `regions H { … }` is one too: its body is made from its regions (`Regions::body`), and
    /// `regions` keeps them, for a backend that walks them (a CPU's micro-lexers) instead.
    Rounds { n: Option<usize>, settle: bool, body: Vec<Stmt>, regions: Option<Regions> },
    Carry(String, Expr),
    Next(String, Expr),
    Ucode { over: Expr, classes: Vec<(String, UClass)> },
    /// `token K = start S [through T | last L]`: a token at each bit of `S`, running through
    /// `T`'s bytes, or ending at the first bit of `L` at or after its start (a tier-1 mask: no
    /// fill, docs/final-shape/decisions/0071).
    Token { kind: String, start: Expr, through: Option<Expr>, last: Option<Expr> },
    Munch { kind: String, over: Expr, lits: Vec<(Vec<u8>, Option<String>)> },
    Keywords { kind: String, words: Vec<(Vec<u8>, Option<String>)>, soft: Vec<Vec<u8>>, required: bool },
    Skip(Expr),
    Error(String, Expr),
    /// `escape "WHY" when E`: a construct the lexer does not handle; a file with a bit of `E` goes
    /// to tree-sitter. Unlike an error, it does not say the file is malformed.
    Escape(String, Expr),
    /// `tree NAME { … }`: the statements that say where each node starts and ends
    /// (docs/parser-generators/13-tape-on-the-gpu.md).
    Tree { name: String, body: Vec<Stmt> },
    /// `node KIND [nested] = G`: a node of the kind spans what `G` matches; the first is the
    /// root. `nested`: it may contain itself, and runs on a stack frame (doc 18 §1).
    /// `whole`: the root spans the whole file, from byte 0, and may hold no token.
    Node { kind: String, nested: bool, whole: bool, body: Gram },
    /// `choice NAME [supertype S] = A | B …`: no node of its own; each alternative's node has
    /// supertype `S`.
    /// `choice N [supertype S] [nested] = …`: `nested`, a level of its own, called where it is
    /// used (it may use itself), with no node of its own.
    Choice { name: String, supertype: Option<String>, nested: bool, body: Gram },
    /// `check KIND = G`: each token of the kind (after `keywords`) must match `G` over its bytes,
    /// a grammar whose names are byte classes and whose literals are bytes; one that does not
    /// sends the file to tree-sitter. Tier 2 checks it in the token's arm, as simdjson's stage 2
    /// parses a number (doc 18 §4).
    Check { kind: String, body: Gram },
    /// `extra KIND …`: nodes of these kinds carry the extra flag.
    Extra(Vec<String>),
    /// `mark KIND = E`: a token of no width before the first grammar token that starts at or
    /// after each bit of `E`; several bits before one token are one mark, and marks of several
    /// kinds before one token come in the order they are declared. `, end`: the end of the file
    /// has it too, after the last token.
    Mark { kind: String, at: Expr, end: bool },
    /// `mark KIND = tokens [after E] E …` (decision 0129): a mark before a token that begins a
    /// run of tokens, each one of an element's choices (`|`): a token kind (a name) or a word
    /// (a string), the extras between them skipped. `after`: the token before is one of
    /// `before`'s. The mark is at the first element's token.
    TokenMark { kind: String, before: Vec<TokenElem>, seq: Vec<Vec<TokenElem>> },
    /// `hidden KIND …` (in a tree): marks taken where the grammar can take them and skipped
    /// elsewhere. A mark not hidden, where the grammar can not take it, refuses the file.
    Hidden(Vec<String>),
    /// `refuse MARK in LEVEL …` (in a tree): a hidden mark that refuses the file between a
    /// nested level's first token and its last (`json_bad` in a JSON document's levels).
    Refuse { mark: String, levels: Vec<String> },
    /// `reading NAME = ITEM, …` (in a tree): a way to read a made node (`reread`).
    Reading { name: String, reading: Reading },
    /// `provisional FIELD …` (in a tree): fields a node holds only until a later token decides
    /// what it is; a node that holds one at the file's end refuses the file.
    Provisional(Vec<String>),
    /// `remember CHOICE …` (in a tree): choices of tokens whose token is remembered at its
    /// level; `as CHOICE` (and a reading's `K as CHOICE`) then names that token's kind. A nested
    /// level starts with its caller's and gives it back when it ends.
    Remember(Vec<String>),
    /// `flag F …` (in a tree): flags a level runs with (`with F A`, `unless F A`), clear in the
    /// root's.
    Flags(Vec<String>),
    /// `implied RULE 'A' …` (in a tree): the tokens written `'A'` in rule `RULE`'s own body (not
    /// in the rules it uses) make no leaf: the grammar implies them, so a named-only tape leaves
    /// them out (`docs/parser-generators/32-named-only-rr-tapes.md`). The parser marks each one
    /// `A as IMPLIED`; the statement stays, for the record.
    Implied { rule: String, tokens: Vec<String> },
}

/// The kind `implied` gives a token (`A as IMPLIED`): it makes no leaf.
pub const IMPLIED: &str = "\u{0}implied";
/// The kind `drop` gives the node just made at this level (`relabel _ as GONE`): it goes.
pub const GONE: &str = "\u{0}gone";

/// Each `implied RULE 'A' …` of a tree's body, applied: its tokens in `RULE`'s own body marked
/// `A as IMPLIED`. A token not written there is an error.
fn apply_implied(body: &mut [Stmt]) -> Result<(), String> {
    fn mark(g: &mut Gram, tokens: &[String], found: &mut Vec<bool>) {
        match g {
            Gram::Lit(t) => {
                if let Some(i) = tokens.iter().position(|x| x == t) {
                    found[i] = true;
                    *g = Gram::As(IMPLIED.into(), Box::new(Gram::Lit(t.clone())));
                }
            }
            // A token given a kind of its own, or merged, is a node's: not implied.
            Gram::Sym(_) | Gram::As(..) | Gram::Merge(..) | Gram::Balanced { .. } | Gram::Any(_) => {}
            Gram::Seq(gs) | Gram::Alt(gs) => gs.iter_mut().for_each(|g| mark(g, tokens, found)),
            Gram::Opt(a) | Gram::Star(a) | Gram::Plus(a) | Gram::Field(_, a) | Gram::Retag(_, a) => mark(a, tokens, found),
            Gram::Wrap { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } | Gram::Reread { body, .. } | Gram::Flag { body, .. } | Gram::Unless { body, .. } => mark(body, tokens, found),
        }
    }
    let implied: Vec<(String, Vec<String>)> = body.iter().filter_map(|s| if let Stmt::Implied { rule, tokens } = s { Some((rule.clone(), tokens.clone())) } else { None }).collect();
    for (rule, tokens) in implied {
        let mut found = vec![false; tokens.len()];
        let mut any = false;
        for s in body.iter_mut() {
            match s {
                Stmt::Node { kind: name, body, .. } | Stmt::Choice { name, body, .. } if *name == rule => {
                    any = true;
                    mark(body, &tokens, &mut found);
                }
                _ => {}
            }
        }
        if !any {
            return Err(format!("implied {rule}: no such rule"));
        }
        if let Some(i) = found.iter().position(|f| !f) {
            return Err(format!("implied {rule}: {:?} is not written in it", tokens[i]));
        }
    }
    Ok(())
}

impl Stmt {
    /// Names this statement defines, recursively through `rounds` and `hypotheses` (which do not
    /// open a scope). Blocks and instances define their own namespaces.
    pub fn defines(&self, out: &mut Vec<String>) {
        match self {
            Stmt::Class(n, _) | Stmt::Mask(n, _) | Stmt::Machine(n, _) | Stmt::Carry(n, _) => out.push(n.clone()),
            Stmt::Reflect { name, .. } | Stmt::Count { name, .. } | Stmt::Number(name, _) => out.push(name.clone()),
            Stmt::Rounds { body, .. } | Stmt::Hyp { body, .. } => body.iter().for_each(|s| s.defines(out)),
            Stmt::Ucode { classes, .. } => classes.iter().for_each(|c| out.push(c.0.clone())),
            _ => {}
        }
    }
}

const PUNCT: [&str; 19] = ["->", "-", "..", "(", ")", "[", "]", "{", "}", ",", "=", "|", "&", "^", "~", ".", "*", ":", "+"];

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
            // A node's `also` and `end` may start lines of their own: no statement starts so.
            let next_cont = matches!(next, Some(Tok::P("|" | "&" | "^")) | Some(Tok::Str(_)) | Some(Tok::Int(_)) | Some(Tok::UPlus(_)))
;
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
    /// A token mark's element: choices joined by `|`, each a kind (a name) or a word (a string).
    fn token_choices(&mut self) -> R<Vec<TokenElem>> {
        let mut out = vec![];
        loop {
            match self.peek().cloned() {
                Some(Tok::Str(s)) => {
                    self.i += 1;
                    out.push(TokenElem::Word(s));
                }
                Some(Tok::Ident(n)) => {
                    self.i += 1;
                    out.push(TokenElem::Kind(n));
                }
                _ => return self.err("a token kind or a word"),
            }
            if !self.eat_p("|") {
                return Ok(out);
            }
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

    /// `A | B …` of sequences.
    fn gram(&mut self) -> R<Gram> {
        let mut alts = vec![self.gram_seq()?];
        while self.eat_p("|") {
            alts.push(self.gram_seq()?);
        }
        Ok(if alts.len() == 1 { alts.pop().expect("one") } else { Gram::Alt(alts) })
    }

    /// Items up to a `|`, a closing bracket or the end of the line.
    fn gram_seq(&mut self) -> R<Gram> {
        let mut items = vec![];
        while matches!(self.peek(), Some(Tok::Str(_) | Tok::Ident(_) | Tok::P("(") | Tok::P("["))) {
            // `wrap K F [as M]`: the rest of the sequence is its body.
            if matches!(self.peek(), Some(Tok::Ident(w)) if w == "wrap") {
                self.i += 1;
                let kind = self.ident()?;
                let field = Some(self.ident()?).filter(|f| f != "_");
                let moved = if self.eat_kw("as") { Some(self.kind()?) } else { None };
                let body = Box::new(self.gram_seq()?);
                items.push(Gram::Wrap { kind, field, moved, body });
                break;
            }
            // `relabel F [as M]`: the rest of the sequence is its body.
            if matches!(self.peek(), Some(Tok::Ident(w)) if w == "relabel") {
                self.i += 1;
                let field = Some(self.ident()?).filter(|f| f != "_");
                let moved = if self.eat_kw("as") { Some(self.kind()?) } else { None };
                let inner = if self.eat_kw("inner") { Some(self.ident()?) } else { None };
                let body = Box::new(self.gram_seq()?);
                items.push(Gram::Relabel { field, moved, inner, body });
                break;
            }
            // `reread R`: the rest of the sequence is its body.
            if matches!(self.peek(), Some(Tok::Ident(w)) if w == "reread") {
                self.i += 1;
                let reading = self.ident()?;
                let body = Box::new(self.gram_seq()?);
                items.push(Gram::Reread { reading, body });
                break;
            }
            // `drop`: the node just made at this level goes (a token the grammar implies, made
            // before the token that says so); the rest of the sequence is its body.
            if matches!(self.peek(), Some(Tok::Ident(w)) if w == "drop") {
                self.i += 1;
                let body = Box::new(self.gram_seq()?);
                items.push(Gram::Relabel { field: None, moved: Some(GONE.into()), inner: None, body });
                break;
            }
            // `fold K`: the rest of the sequence is its body.
            if matches!(self.peek(), Some(Tok::Ident(w)) if w == "fold") {
                self.i += 1;
                let kind = self.kind()?;
                let body = Box::new(self.gram_seq()?);
                items.push(Gram::Fold { kind, body });
                break;
            }
            let mut item = self.gram_atom()?;
            loop {
                if self.eat_p("*") {
                    item = Gram::Star(Box::new(item));
                } else if self.eat_p("+") {
                    item = Gram::Plus(Box::new(item));
                } else if self.eat_kw("as") {
                    item = Gram::As(self.kind()?, Box::new(item));
                } else {
                    break;
                }
            }
            items.push(item);
        }
        match items.len() {
            0 => self.err("a token, a name, ( or ["),
            1 => Ok(items.pop().expect("one")),
            _ => Ok(Gram::Seq(items)),
        }
    }

    fn gram_atom(&mut self) -> R<Gram> {
        if self.eat_p("(") {
            let g = self.gram()?;
            self.want_p(")")?;
            return Ok(g);
        }
        if self.eat_p("[") {
            let g = self.gram()?;
            self.want_p("]")?;
            return Ok(Gram::Opt(Box::new(g)));
        }
        if let Some(Tok::Str(_)) = self.peek() {
            return Ok(Gram::Lit(self.kind()?));
        }
        let name = self.ident()?;
        if self.eat_p(":") {
            return Ok(Gram::Field(name, Box::new(self.gram_atom()?)));
        }
        match name.as_str() {
            "with" | "without" => {
                let flag = self.ident()?;
                Ok(Gram::Flag { flag, set: name == "with", body: Box::new(self.gram_atom()?) })
            }
            "unless" => {
                let flag = self.ident()?;
                Ok(Gram::Unless { flag, body: Box::new(self.gram_atom()?) })
            }
            "retag" => {
                let kind = self.ident()?;
                Ok(Gram::Retag(kind, Box::new(self.gram_atom()?)))
            }
            "merge" => {
                let kind = self.kind()?;
                Ok(Gram::Merge(kind, Box::new(self.gram_atom()?)))
            }
            "any" => {
                let mut except = vec![];
                if self.eat_kw("except") {
                    while matches!(self.peek(), Some(Tok::Str(_))) {
                        except.push(self.kind()?);
                    }
                    if except.is_empty() {
                        return self.err("any except 'A' …");
                    }
                }
                Ok(Gram::Any(except))
            }
            "balanced" => {
                let kind = self.kind()?;
                let open = self.kind()?;
                let close = self.kind()?;
                Ok(Gram::Balanced { kind, open, close })
            }
            _ => Ok(Gram::Sym(name)),
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
    /// A number: terms joined by `-`, left to right.
    fn nexpr(&mut self) -> R<NExpr> {
        let mut l = self.nterm()?;
        while self.eat_p("-") {
            l = NExpr::Sub(Box::new(l), Box::new(self.nterm()?));
        }
        Ok(l)
    }
    fn nterm(&mut self) -> R<NExpr> {
        if self.eat_p("(") {
            let e = self.nexpr()?;
            self.want_p(")")?;
            return Ok(e);
        }
        if self.eat_kw("position") {
            return Ok(NExpr::Position);
        }
        let name = self.ident()?;
        if (name == "fill" || name == "back_fill") && self.eat_p("(") {
            let s = self.expr()?;
            self.want_p(",")?;
            let e = self.expr()?;
            self.want_p(",")?;
            let v = self.nexpr()?;
            self.want_p(")")?;
            return Ok(NExpr::Fill { s, e, v: Box::new(v), back: name == "back_fill" });
        }
        Ok(NExpr::Name(name))
    }
    fn states(&mut self) -> R<Vec<String>> {
        let mut v = vec![self.ident()?];
        while self.eat_p("|") {
            v.push(self.ident()?);
        }
        Ok(v)
    }

    // --- statements ---
    /// `regions H { K = O .. C [bytes N] [outside] [else E bytes M] … }` (`Regions`).
    fn regions(&mut self) -> R<Regions> {
        let hidden = self.ident()?;
        self.want_p("{")?;
        self.nls();
        let mut kinds = Vec::new();
        while !self.eat_p("}") {
            let kind = self.ident()?;
            self.want_p("=")?;
            let opener = self.expr()?;
            self.want_p("..")?;
            let closer = self.expr()?;
            let bytes = if self.eat_kw("bytes") { self.int()? } else { 1 };
            let outside = self.eat_kw("outside");
            let empty = if self.eat_kw("else") {
                let e = self.ident()?;
                self.want_kw("bytes")?;
                Some((e, self.int()?))
            } else {
                None
            };
            if !(1..=3).contains(&bytes) || (outside && bytes != 1) || empty.as_ref().is_some_and(|e| !(1..=3).contains(&e.1)) {
                return self.err("a closer of 1 to 3 bytes (`outside`: 1), an `else` of 1 to 3");
            }
            kinds.push(Region { kind, opener, closer, bytes: bytes as u8, outside, empty: empty.map(|(e, m)| (e, m as u8)) });
            self.nls();
        }
        if kinds.is_empty() {
            return self.err("a region");
        }
        Ok(Regions { hidden, kinds })
    }

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
            "number" => {
                let n = self.ident()?;
                self.want_p("=")?;
                Stmt::Number(n, self.nexpr()?)
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
                let n = if self.eat_kw("until") {
                    self.want_kw("settled")?;
                    return Ok(Stmt::Rounds { n: None, settle: true, body: self.block()?, regions: None });
                } else {
                    self.int()? as usize
                };
                let settle = self.eat_kw("until");
                if settle {
                    self.want_kw("settled")?;
                }
                Stmt::Rounds { n: Some(n), settle, body: self.block()?, regions: None }
            }
            "regions" => {
                let regions = self.regions()?;
                Stmt::Rounds { n: None, settle: true, body: regions.body(), regions: Some(regions) }
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
            "escape" => {
                let m = String::from_utf8_lossy(&self.string()?).into_owned();
                self.want_kw("when")?;
                Stmt::Escape(m, self.expr()?)
            }
            "tree" => {
                let name = self.ident()?;
                let mut body = self.block()?;
                apply_implied(&mut body)?;
                Stmt::Tree { name, body }
            }
            "implied" => {
                let rule = self.ident()?;
                let mut tokens = vec![];
                while let Some(Tok::Str(s)) = self.peek() {
                    tokens.push(String::from_utf8_lossy(s).into_owned());
                    self.i += 1;
                }
                if tokens.is_empty() {
                    return self.err("implied RULE 'A' …");
                }
                Stmt::Implied { rule, tokens }
            }
            "node" => {
                let kind = self.kind()?;
                let nested = self.eat_kw("nested");
                let whole = self.eat_kw("whole");
                self.want_p("=")?;
                Stmt::Node { kind, nested, whole, body: self.gram()? }
            }
            "choice" => {
                let name = self.ident()?;
                let supertype = if self.eat_kw("supertype") { Some(self.ident()?) } else { None };
                let nested = self.eat_kw("nested");
                self.want_p("=")?;
                Stmt::Choice { name, supertype, nested, body: self.gram()? }
            }
            "check" => {
                let kind = self.kind()?;
                self.want_p("=")?;
                Stmt::Check { kind, body: self.gram()? }
            }
            "refuse" => {
                let mark = self.kind()?;
                if !self.eat_kw("in") {
                    return self.err("refuse MARK in LEVEL …");
                }
                let mut levels = vec![];
                while matches!(self.peek(), Some(Tok::Ident(_))) {
                    levels.push(self.ident()?);
                }
                if levels.is_empty() {
                    return self.err("refuse MARK in LEVEL …");
                }
                Stmt::Refuse { mark, levels }
            }
            "reading" => {
                let name = self.ident()?;
                self.want_p("=")?;
                let mut reading = Reading::default();
                loop {
                    if self.eat_kw("field") {
                        let f = self.ident()?;
                        self.want_kw("as")?;
                        reading.fields.push((f, self.ident()?));
                    } else if self.eat_kw("root") {
                        if reading.root.is_some() {
                            return self.err("a reading's root twice");
                        }
                        reading.root = Some(if self.eat_kw("gone") {
                            ReadKind::Gone
                        } else {
                            self.want_kw("as")?;
                            ReadKind::As(self.kind()?)
                        });
                    } else if self.eat_kw("keep") {
                        let mut any = false;
                        while matches!(self.peek(), Some(Tok::Ident(_) | Tok::Str(_))) {
                            reading.kinds.push((self.kind()?, ReadKind::Keep));
                            any = true;
                        }
                        if !any {
                            return self.err("keep K …");
                        }
                    } else {
                        let k = self.kind()?;
                        let what = if self.eat_kw("gone") {
                            ReadKind::Gone
                        } else if self.eat_kw("as") {
                            ReadKind::As(self.kind()?)
                        } else {
                            ReadKind::As(k.clone())
                        };
                        reading.kinds.push((k, what));
                    }
                    if !self.eat_p(",") {
                        break;
                    }
                }
                Stmt::Reading { name, reading }
            }
            "flag" => {
                let mut names = vec![];
                while matches!(self.peek(), Some(Tok::Ident(_))) {
                    names.push(self.ident()?);
                }
                if names.is_empty() {
                    return self.err("flag F …");
                }
                Stmt::Flags(names)
            }
            "remember" => {
                let mut names = vec![];
                while matches!(self.peek(), Some(Tok::Ident(_))) {
                    names.push(self.ident()?);
                }
                if names.is_empty() {
                    return self.err("remember CHOICE …");
                }
                Stmt::Remember(names)
            }
            "provisional" => {
                let mut fields = vec![];
                while matches!(self.peek(), Some(Tok::Ident(_))) {
                    fields.push(self.ident()?);
                }
                if fields.is_empty() {
                    return self.err("provisional FIELD …");
                }
                Stmt::Provisional(fields)
            }
            "extra" | "hidden" => {
                let mut kinds = vec![];
                while matches!(self.peek(), Some(Tok::Ident(_) | Tok::Str(_))) {
                    kinds.push(self.kind()?);
                }
                if kw == "extra" { Stmt::Extra(kinds) } else { Stmt::Hidden(kinds) }
            }
            "mark" => {
                let kind = self.kind()?;
                self.want_p("=")?;
                if self.eat_kw("tokens") {
                    let before = if self.eat_kw("after") { self.token_choices()? } else { vec![] };
                    let mut seq = vec![];
                    while matches!(self.peek(), Some(Tok::Ident(_) | Tok::Str(_))) {
                        seq.push(self.token_choices()?);
                    }
                    if seq.is_empty() {
                        return self.err("a token after `tokens`");
                    }
                    let s = Stmt::TokenMark { kind, before, seq };
                    self.end_stmt()?;
                    return Ok(s);
                }
                let at = self.expr()?;
                let end = self.eat_p(",");
                if end {
                    self.want_kw("end")?;
                }
                Stmt::Mark { kind, at, end }
            }
            "token" => {
                let kind = self.kind()?;
                self.want_p("=")?;
                self.want_kw("start")?;
                let start = self.expr()?;
                let through = if self.eat_kw("through") { Some(self.expr()?) } else { None };
                let last = if through.is_none() && self.eat_kw("last") { Some(self.expr()?) } else { None };
                Stmt::Token { kind, start, through, last }
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

#[cfg(test)]
mod tier2_tests {
    use super::*;

    /// The tier-2 forms of decision 0071, as JSON's tree section writes them.
    #[test]
    fn tree_grammar_parses() {
        let src = r#"
lexer t
tree json {
  node document               = value
  node object nested          = '{' [pair (',' pair)*] '}'
  node pair                   = key: string ':' value: value
  node array nested           = '[' [value (',' value)*] ']'
  node string                 = '"' (string_content | escape_sequence)* '"'
  choice value supertype _value = object | array | string | number | true | false | null
  node list = item+ | '(' ')'
}
"#;
        let stmts = parse(src).expect("parses");
        let Some(Stmt::Tree { body, .. }) = stmts.iter().find(|s| matches!(s, Stmt::Tree { .. })) else { panic!("a tree") };
        let lit = |s: &str| Gram::Lit(s.to_owned());
        let sym = |s: &str| Gram::Sym(s.to_owned());
        assert!(matches!(&body[0], Stmt::Node { kind, nested: false, body: Gram::Sym(v), .. } if kind == "document" && v == "value"));
        let Stmt::Node { nested: true, body: object, .. } = &body[1] else { panic!("object: {:?}", body[1]) };
        let pairs = Gram::Seq(vec![sym("pair"), Gram::Star(Box::new(Gram::Seq(vec![lit(","), sym("pair")])))]);
        assert_eq!(object, &Gram::Seq(vec![lit("{"), Gram::Opt(Box::new(pairs)), lit("}")]));
        let Stmt::Node { body: pair, .. } = &body[2] else { panic!("pair") };
        assert_eq!(pair, &Gram::Seq(vec![Gram::Field("key".into(), Box::new(sym("string"))), lit(":"), Gram::Field("value".into(), Box::new(sym("value")))]));
        let Stmt::Node { body: string, .. } = &body[4] else { panic!("string") };
        assert_eq!(string, &Gram::Seq(vec![lit("\""), Gram::Star(Box::new(Gram::Alt(vec![sym("string_content"), sym("escape_sequence")]))), lit("\"")]));
        let Stmt::Choice { name, supertype: Some(s), body: Gram::Alt(alts), .. } = &body[5] else { panic!("value: {:?}", body[5]) };
        assert_eq!((name.as_str(), s.as_str(), alts.len()), ("value", "_value", 7));
        let Stmt::Node { body: Gram::Alt(alts), .. } = &body[6] else { panic!("list") };
        assert_eq!(alts, &vec![Gram::Plus(Box::new(sym("item"))), Gram::Seq(vec![lit("("), lit(")")])]);
    }

    /// `check KIND = G`, a grammar over a token's bytes; the evaluator refuses a token it does
    /// not match (doc 18 §4).
    /// Numbers: a byte's place, carried back from a run's last byte, a difference, a fill
    /// that restarts at each of its starts, and the compares, with a constant too.
    #[test]
    fn numbers_evaluate_per_byte() {
        let src = "lexer t\nclass tick = '`'\nmask last = tick & ~next(tick)\nnumber end = back_fill(last, ~tick, position)\nnumber rem = end - position\nmask long = tick & at_least(rem, 3)\nnumber n = fill(long, long, rem)\nmask close = tick & at_least(rem, n) & ~long\nmask over = above(n, rem)\n";
        let stmts = parse(src).expect("parses");
        let tables = std::collections::HashMap::new();
        let text = b"a````b``c`````d";
        let out = crate::eval::Ev::run(&stmts, text, &tables).expect("runs");
        let num = |k: &str| match &out.vars[k] {
            crate::eval::Val::N(v) => v.clone(),
            _ => panic!("{k}: not a number"),
        };
        let ones = |k: &str| match &out.vars[k] {
            crate::eval::Val::M(m) => m.ones_iter().collect::<Vec<usize>>(),
            _ => panic!("{k}: not a mask"),
        };
        assert_eq!(num("end"), [0, 4, 4, 4, 4, 0, 7, 7, 0, 13, 13, 13, 13, 13, 0]);
        // Off the runs, `end` is 0: the difference wraps, mod 2^32.
        let w = |k: u32| 0u32.wrapping_sub(k);
        assert_eq!(num("rem"), [0, 3, 2, 1, 0, w(5), 1, 0, w(8), 4, 3, 2, 1, 0, w(14)]);
        assert_eq!(ones("long"), [1, 9, 10]);
        assert_eq!(num("n"), [0, 3, 3, 3, 3, 3, 3, 3, 3, 4, 3, 3, 3, 3, 3]);
        assert_eq!(ones("close"), Vec::<usize>::new(), "no run after a long one as long as it");
        assert!(stmts.iter().any(|s| matches!(s, Stmt::Number(n, NExpr::Sub(..)) if n == "rem")));
        assert_eq!(ones("over"), [2, 3, 4, 6, 7, 11, 12, 13]);
        let bad = "lexer t\nclass q = '\"'\nrounds until settled {\n  carry h = empty\n  number x = back_fill(q, h, position)\n  next h = prev(at_least(x, 1))\n}\n";
        let stmts = parse(bad).expect("parses");
        assert!(strictly_causal(match &stmts[2] {
            Stmt::Rounds { body, .. } => body,
            _ => panic!("rounds"),
        })
        .is_err());
    }

    #[test]
    fn check_matches_a_tokens_bytes() {
        let src = "lexer t\nclass digit = '0'..'9'\nclass nonzero = '1'..'9'\nclass ws = ' '\nmask word = ~ws\ntoken number = start starts(word) last lasts(word)\nskip ws\ncheck number = ['-'] ('0' | nonzero digit*) ['.' digit*]\n";
        let stmts = parse(src).expect("parses");
        let tables = std::collections::HashMap::new();
        let errors = |text: &str| crate::eval::Ev::run(&stmts, text.as_bytes(), &tables).expect("runs").errors.len();
        assert_eq!([errors("0 -12 3. 4.50"), errors("01"), errors("-"), errors("1x"), errors("1..2")], [0, 1, 1, 1, 1]);
    }
}

/// Whether a `rounds` body is strictly causal: each `next X = E` reaches the carries only
/// through a `prev` (its byte `p` reads them at bytes before `p`), and no `next(…)` reads a
/// value that a carry reaches. Then a round is right on every byte the round before was, and on
/// one more: the rounds settle, at most one per byte, and the fixed point is the one value
/// every byte's left context gives (docs/parser-generators/03-bitlex.md §2.7).
pub fn strictly_causal(body: &[Stmt]) -> Result<(), String> {
    use std::collections::{HashMap, HashSet};
    let carries: HashSet<String> = body.iter().filter_map(|s| if let Stmt::Carry(n, _) = s { Some(n.clone()) } else { None }).collect();
    // Per name the body defines: the carries it reaches at its own byte or after (`loose`),
    // and those it reaches at all.
    let mut loose: HashMap<String, HashSet<String>> = HashMap::new();
    let mut any: HashMap<String, HashSet<String>> = HashMap::new();
    fn walk(e: &Expr, carries: &HashSet<String>, loose: &HashMap<String, HashSet<String>>, any: &HashMap<String, HashSet<String>>) -> Result<(HashSet<String>, HashSet<String>), String> {
        let union = |xs: Vec<(HashSet<String>, HashSet<String>)>| {
            xs.into_iter().fold((HashSet::new(), HashSet::new()), |(mut l, mut a), (x, y)| {
                l.extend(x);
                a.extend(y);
                (l, a)
            })
        };
        let arg = |a: &Arg| match a {
            Arg::E(e) => walk(e, carries, loose, any),
            _ => Ok((HashSet::new(), HashSet::new())),
        };
        Ok(match e {
            Expr::Name(n) if carries.contains(n) => (HashSet::from([n.clone()]), HashSet::from([n.clone()])),
            Expr::Name(n) => (loose.get(n).cloned().unwrap_or_default(), any.get(n).cloned().unwrap_or_default()),
            Expr::Empty => (HashSet::new(), HashSet::new()),
            Expr::Not(a) => walk(a, carries, loose, any)?,
            Expr::And(a, b) | Expr::Or(a, b) | Expr::Xor(a, b) => union(vec![walk(a, carries, loose, any)?, walk(b, carries, loose, any)?]),
            Expr::Call(f, args) => {
                let parts = union(args.iter().map(arg).collect::<Result<_, _>>()?);
                match f.as_str() {
                    // `prev(a, k)`, k at least 1: byte `p` reads `a` before `p`.
                    "prev" => (HashSet::new(), parts.1),
                    "next" if !parts.1.is_empty() => return Err(format!("`next` of a value a carry reaches ({})", sorted(&parts.1))),
                    _ => parts,
                }
            }
            Expr::Test { target, arg: a, .. } => {
                let t = (loose.get(target).cloned().unwrap_or_default(), any.get(target).cloned().unwrap_or_default());
                let x = match a {
                    Some(a) => walk(a, carries, loose, any)?,
                    None => (HashSet::new(), HashSet::new()),
                };
                union(vec![t, x])
            }
        })
    }
    fn sorted(s: &HashSet<String>) -> String {
        let mut v: Vec<&String> = s.iter().collect();
        v.sort();
        v.iter().map(|x| x.as_str()).collect::<Vec<_>>().join(", ")
    }
    for s in body {
        match s {
            Stmt::Mask(n, e) => {
                let (l, a) = walk(e, &carries, &loose, &any)?;
                loose.insert(n.clone(), l);
                any.insert(n.clone(), a);
            }
            Stmt::Number(name, e) => {
                type Reach = (HashSet<String>, HashSet<String>);
                fn nwalk(e: &NExpr, w: &dyn Fn(&Expr) -> Result<Reach, String>, loose: &HashMap<String, HashSet<String>>, any: &HashMap<String, HashSet<String>>) -> Result<Reach, String> {
                    Ok(match e {
                        NExpr::Position => (HashSet::new(), HashSet::new()),
                        NExpr::Name(n) => (loose.get(n).cloned().unwrap_or_default(), any.get(n).cloned().unwrap_or_default()),
                        NExpr::Fill { back: true, .. } => return Err("a `back_fill` in `rounds until settled`: it reads the bytes after".into()),
                        NExpr::Fill { s, e, v, .. } => {
                            let mut out = nwalk(v, w, loose, any)?;
                            for x in [s, e] {
                                let (l, a) = w(x)?;
                                out.0.extend(l);
                                out.1.extend(a);
                            }
                            out
                        }
                        NExpr::Sub(a, b) => {
                            let (mut l, mut x) = nwalk(a, w, loose, any)?;
                            let (l2, x2) = nwalk(b, w, loose, any)?;
                            l.extend(l2);
                            x.extend(x2);
                            (l, x)
                        }
                    })
                }
                let (l, a) = nwalk(e, &|x| walk(x, &carries, &loose, &any), &loose, &any)?;
                loose.insert(name.clone(), l);
                any.insert(name.clone(), a);
            }
            Stmt::Reflect { name, a, b, reset, .. } => {
                let mut l = HashSet::new();
                let mut all = HashSet::new();
                for e in [Some(a), Some(b), reset.as_ref()].into_iter().flatten() {
                    let (x, y) = walk(e, &carries, &loose, &any)?;
                    l.extend(x);
                    all.extend(y);
                }
                loose.insert(name.clone(), l);
                any.insert(name.clone(), all);
            }
            Stmt::Next(n, e) => {
                if !carries.contains(n) {
                    return Err(format!("next {n}: no carry of that name"));
                }
                let (l, _) = walk(e, &carries, &loose, &any)?;
                if !l.is_empty() {
                    return Err(format!("next {n} reaches {} with no `prev` on the way: the rounds may not settle", sorted(&l)));
                }
            }
            Stmt::Carry(..) => {}
            other => return Err(format!("in `rounds until settled`, only masks, scans, carries and nexts: {other:?}")),
        }
    }
    Ok(())
}
