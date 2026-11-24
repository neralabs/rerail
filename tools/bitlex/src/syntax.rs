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
/// After the first element, three more: `~C` (`Not`), a token none of `C`'s choices, never the
/// file's end, the only choice of its element; `end` (`End`), no token left, in the last element;
/// `(C)*` (`Many`, decision 0416), any number of tokens of `C`'s choices (or of none of them, for
/// a `C` that is one `~`), then the elements after
/// it from any of them on (as a forward scan for the first place they hold), the only choice of
/// its element, one a run and never its last. A run's `nest` counts depths (`Nest`); its `ending
/// (C)` (the third field) tries the elements after it only where its last token, or the token
/// before it where it took none, is one of `C` (Q342).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenElem {
    Kind(String),
    Word(Vec<u8>),
    Not(Vec<TokenElem>),
    End,
    Many(Vec<TokenElem>, Vec<Nest>, Vec<TokenElem>),
    /// `E without M`, among an element's choices: the element holds only at a token without
    /// mark `M`'s bit (a mark of a mask, its bits lifted to the next grammar token, its `unless
    /// after` applied: the bit a token's record holds), as the lean parser asks
    /// `!peek_at(j).nl`.
    Without(String),
    /// `E with M`: the element holds only at a token with mark `M`'s bit, as the lean parser
    /// asks `peek_at(j).nl`.
    With(String),
    /// `E starting "B"`, or `starting "B"` alone (any token): the element holds only at a
    /// token whose first byte is one of `B`'s, as the lean lexer's `asi` reads the byte after a
    /// comment. A gap pattern's only (`GapPattern`).
    Starting(Vec<u8>),
}

/// A mark's `drops`, `loses` or `refuses` question (decision 0451): `, drops [unless] [after A
/// …] before B …` (`before` may go where `after` has an element), over the records as the lexer wrote them, extras included. `after`'s elements are the
/// records before the token, the last the extra pending last (its `with M`: the marks of its
/// own gap, `Records::extra`); `before`'s run from the token on. The question holds where none
/// of its `unless` entries does and one of the others does, as the lean lexer's `asi` decides
/// from the bytes after a line break.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GapPattern {
    pub unless: bool,
    /// `since A …` in place of `after`: `A` is the grammar token before the token (none before
    /// the file's first), and the elements after it the records from it on, each before the
    /// token: the first comment after a line break where the lean lexer's `asi` decides, however
    /// many follow it.
    pub since: bool,
    pub after: Vec<Vec<TokenElem>>,
    pub before: Vec<Vec<TokenElem>>,
}

/// A depth a run counts (`(C)* nest (O to C) …`, Q339), as the lean parsers' forward scans count
/// one (`type_args_end`, `lambda_params_end`): a token of the run among `open`'s choices goes one
/// deeper, one among `close`'s one out. The elements after the run are tried only where every
/// depth is 0, and a closer where its depth is 0 ends the run: it fails there unless they hold.
/// Kinds and words only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nest {
    pub open: Vec<TokenElem>,
    pub close: Vec<TokenElem>,
}

#[derive(Clone, Debug)]
pub enum Arg {
    E(Expr),
    N(u64),
    S(Vec<u8>),
}

/// A test of a token over its own bytes (`Stmt::TokenEscape`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenTest {
    /// The mask's bits in the token.
    Count,
    /// The mask at each of the token's first bytes, from its first.
    Run,
    /// The mask at every byte of the token, the token that long at least: `all(digit) at
    /// least 19`, an integer's token of 19 digits or more, as a lexer knows the number it read
    /// had no `.`.
    All,
}

/// A mark's token test (`Stmt::Mark`'s `at` and `after`): at tokens of `kind`, `test` over
/// `expr` at least `least`; one that passes is a bit of the mark at its first byte, or (`after`)
/// at the byte after its last.
#[derive(Clone, Debug)]
pub struct MarkCheck {
    pub after: bool,
    pub kind: String,
    pub test: TokenTest,
    pub expr: Expr,
    pub least: u32,
}

/// How a token ends, told at its start (`Stmt::Token`'s `run` and `pair`).
#[derive(Clone, Debug)]
pub enum TokenRule {
    /// Its first byte, then the bytes of the mask after it, in a row.
    Run(Expr),
    /// `run R once D`: a run of `R` with one bit of `D` in it at most; the second ends the token
    /// before it, as a lexer reads a number's one `.` and stops at the next (`1.2.3` is `1.2`,
    /// `.`, `3`). Every loop cuts the run there (`TokenCheck`'s cut); the mask form below.
    RunOnce(Expr, Expr),
    /// One byte, or two where the mask holds at its first.
    Pair(Expr),
    /// `pair P`, `P` a `pairs P = "AB" … "ABC" …`: one byte, or two where its first and the
    /// next are one of the pairs, or three where those and the byte after are one of the
    /// triples, as a lexer reads its next bytes; `P` its mask, `three` its triples' starts
    /// (`P_three`), where it has triples.
    Pairs { pairs: Vec<Vec<u8>>, mask: Expr, three: Option<Expr> },
    /// `read K | …`: a region's token, read from its opener to its closer (decision 0177): the
    /// region whose opener holds at its first byte, then its closer's first byte after it; its
    /// last byte the closer's last (`outside`: the byte before it). A lexer's reader of a
    /// string or a comment: on the CPU, a search of the closer's bits from the start.
    Read(Vec<Region>),
    /// `read K parts { P = E bytes N, … } else C`: a region read in parts, as a lexer reads a
    /// string with escapes. Its tokens are the opener's byte and the closer's bytes (this kind),
    /// each part `P` (`N` bytes from a bit of `E` in the region), and each run of the region's
    /// other bytes (`C`). The parts' kinds are tokens of their own (`Inside`, after this one).
    /// `read K | L parts …`: regions of one `regions` that one reader reads alike, as Kotlin's
    /// strings and raw strings are, at one byte; each closer its own region's length.
    ReadParts(Vec<Region>, Parts),
    /// A token of a region read in parts (`ReadParts`): `last` its last-byte mask. The reader
    /// starts it, not the token loop: no first byte ends it by a rule. With the reader's kind
    /// (its opener's), and whether it closes the region: its tokens come only between the two.
    Inside(Expr, String, bool),
}

/// A region's parts (`TokenRule::ReadParts`): each part's kind, start mask and bytes, and the
/// kind of the bytes between them.
#[derive(Clone, Debug)]
pub struct Parts {
    pub parts: Vec<(String, Expr, PartLen)>,
    pub content: String,
    /// `closes { K = E bytes N }`: a closer where `E` holds is a token of kind `K`, `N` bytes
    /// from the closer's first; the others are the reader's kind (Kotlin's string, closed by
    /// a `"` or by a template's `${`).
    pub closes: Vec<(String, Expr, u8)>,
}

/// How far a part of a region read in parts runs (`Parts`): `bytes N`, or `last L`, through the
/// first byte of `L` at or after its start (an identifier after Kotlin's `$`).
#[derive(Clone, Debug)]
pub enum PartLen {
    Bytes(u8),
    Last(Expr),
}

impl Parts {
    /// The token rows of a region `r` read in parts from its opener `s` (one byte, the reader's
    /// own row), as masks: the closer, of the reader's kind (`kind`), then each part's and the
    /// content's. With `f` the region's fill and `body = f & ~s`,
    ///
    /// ```text
    /// closer:  c = prev(f) & ~f                    # its first byte; its last N - 1 after
    /// part P:  ps = E & body;   last prev(ps, N - 1)
    ///          (`last L`: its bytes fill(ps, prev(L)), its last L & those bytes)
    /// content: x = body & ~(each part's bytes);   start x & ~prev(x), last x & ~next(x)
    /// ```
    /// With `closes`: the closers' last bytes, as one mask (`ReadArm::last`); `None`, every
    /// closer the region's length.
    pub fn closer_last(&self, rs: &[Region]) -> Option<Expr> {
        if self.closes.is_empty() && rs.iter().all(|r| r.bytes == rs[0].bytes && r.or.is_none()) {
            return None;
        }
        let and = |a: Expr, b: Expr| Expr::And(Box::new(a), Box::new(b));
        let closer = Self::closer(rs);
        let mut last: Option<Expr> = None;
        for (_, e, n) in &self.closes {
            let c = prev(and(closer.clone(), e.clone()), n - 1);
            last = Some(or_of(last, c));
        }
        for r in rs {
            let own = self.own(r);
            last = Some(or_of(last, r.closer_last(own)));
        }
        last
    }

    /// The regions' fill: `f_K`, or each's, `|`ed.
    fn fill(rs: &[Region]) -> Expr {
        rs.iter().fold(None, |a, r| Some(or_of(a, Expr::Name(format!("f_{}", r.kind))))).expect("a region")
    }

    /// The closers' first bytes: the byte after each fill.
    fn closer(rs: &[Region]) -> Expr {
        let f = Self::fill(rs);
        Expr::And(Box::new(prev(f.clone(), 1)), Box::new(Expr::Not(Box::new(f))))
    }

    /// Region `r`'s closers that are the reader's own kind (no `closes` mask holds).
    fn own(&self, r: &Region) -> Expr {
        let and = |a: Expr, b: Expr| Expr::And(Box::new(a), Box::new(b));
        let not = |a: Expr| Expr::Not(Box::new(a));
        let mut own = Self::closer(std::slice::from_ref(r));
        for (_, e, _) in &self.closes {
            own = and(own, not(e.clone()));
        }
        own
    }

    /// The rows of each region (`rs`), in turn: each region's tokens are rows of their own, so
    /// each row is in its region's mode (`row_modes`), as a reader of one region's are.
    pub fn rows(&self, kind: &str, rs: &[Region], s: &Expr) -> Vec<Stmt> {
        rs.iter().flat_map(|r| self.region_rows(kind, r, s)).collect()
    }

    fn region_rows(&self, kind: &str, r: &Region, s: &Expr) -> Vec<Stmt> {
        let and = |a: Expr, b: Expr| Expr::And(Box::new(a), Box::new(b));
        let or = |a: Expr, b: Expr| Expr::Or(Box::new(a), Box::new(b));
        let not = |a: Expr| Expr::Not(Box::new(a));
        let next = |e: Expr| Expr::Call("next".into(), vec![Arg::E(e)]);
        let f = Expr::Name(format!("f_{}", r.kind));
        let body = and(f.clone(), not(s.clone()));
        let closer = Self::closer(std::slice::from_ref(r));
        let reader = kind;
        let closing = |kind: &str, start: Expr, last: Expr, closes: bool| Stmt::Token { kind: kind.to_owned(), start, through: None, last: Some(last.clone()), rule: Some(TokenRule::Inside(last, reader.to_owned(), closes)) };
        let token = |kind: &str, start: Expr, last: Expr| closing(kind, start, last, false);
        // The reader's own row is its token's, the opener; the closer is its kind's too, but
        // where a `closes` mask holds: that one's kind's, its bytes.
        let mut out = vec![];
        for (k, e, n) in &self.closes {
            let c = and(closer.clone(), e.clone());
            out.push(closing(k, c.clone(), prev(c, n - 1), true));
        }
        let own = self.own(r);
        out.insert(0, closing(kind, own.clone(), r.closer_last(own), true));
        let mut covered: Option<Expr> = None;
        let mut cover = |c: Expr| {
            covered = Some(match covered.take() {
                Some(x) => or(x, c),
                None => c,
            });
        };
        for (p, e, len) in &self.parts {
            let ps = and(e.clone(), body.clone());
            match len {
                PartLen::Bytes(n) => {
                    out.push(token(p, ps.clone(), prev(ps.clone(), n - 1)));
                    for k in 0..*n {
                        cover(prev(ps.clone(), k));
                    }
                }
                PartLen::Last(l) => {
                    let bytes = Expr::Call("fill".into(), vec![Arg::E(ps.clone()), Arg::E(prev(l.clone(), 1))]);
                    out.push(token(p, ps.clone(), and(l.clone(), bytes.clone())));
                    cover(bytes);
                }
            }
        }
        let x = match covered {
            Some(c) => and(body, not(c)),
            None => body,
        };
        out.push(token(&self.content, and(x.clone(), not(prev(x.clone(), 1))), and(x.clone(), not(next(x)))));
        out
    }
}

/// `prev(e, k)`, `e` itself at 0.
fn prev(e: Expr, k: u8) -> Expr {
    match k {
        0 => e,
        1 => Expr::Call("prev".into(), vec![Arg::E(e)]),
        k => Expr::Call("prev".into(), vec![Arg::E(e), Arg::N(u64::from(k))]),
    }
}

/// `a | e`, or `e` with no `a`.
fn or_of(a: Option<Expr>, e: Expr) -> Expr {
    match a {
        Some(a) => Expr::Or(Box::new(a), Box::new(e)),
        None => e,
    }
}

impl TokenRule {
    /// The token's last byte as a mask, from its start `s`:
    ///
    /// ```text
    /// run R:  t = s | fill(prev(s) & R, ~R);   last = t & ~next(t & ~s)
    /// run R once D: the run's bytes from the second `D` after a start on cut, each token's:
    ///         d = t & D & ~s;  one = fill(prev(d) & ~s, ~t | s);
    ///         t' = t & ~fill(d & one, ~t | s);   last = t' & ~next(t' & ~s)
    /// pair P: last = (s & ~P) | prev(s & P)
    /// ```
    pub fn last(&self, s: &Expr) -> Expr {
        let and = |a: Expr, b: Expr| Expr::And(Box::new(a), Box::new(b));
        let or = |a: Expr, b: Expr| Expr::Or(Box::new(a), Box::new(b));
        let not = |a: Expr| Expr::Not(Box::new(a));
        let call = |f: &str, args: Vec<Expr>| Expr::Call(f.to_string(), args.into_iter().map(Arg::E).collect());
        match self {
            TokenRule::Run(r) => {
                let after = call("fill", vec![and(call("prev", vec![s.clone()]), r.clone()), not(r.clone())]);
                let t = or(s.clone(), after);
                and(t.clone(), not(call("next", vec![and(t, not(s.clone()))])))
            }
            TokenRule::RunOnce(r, d) => {
                let after = call("fill", vec![and(call("prev", vec![s.clone()]), r.clone()), not(r.clone())]);
                let t = or(s.clone(), after);
                // A start, or a byte past the run, ends what the fills carry: each token's own.
                let stop = or(not(t.clone()), s.clone());
                let ds = and(and(t.clone(), d.clone()), not(s.clone()));
                let one = call("fill", vec![and(call("prev", vec![ds.clone()]), not(s.clone())), stop.clone()]);
                let cut = call("fill", vec![and(ds, one), stop]);
                let t = and(t, not(cut));
                and(t.clone(), not(call("next", vec![and(t, not(s.clone()))])))
            }
            TokenRule::Pair(p) | TokenRule::Pairs { mask: p, three: None, .. } => or(and(s.clone(), not(p.clone())), call("prev", vec![and(s.clone(), p.clone())])),
            // A triple's last: two after its start; a pair's, one.
            TokenRule::Pairs { mask: p, three: Some(t), .. } => {
                let two = call("prev", vec![and(and(s.clone(), p.clone()), not(t.clone()))]);
                let three = Expr::Call("prev".into(), vec![Arg::E(and(s.clone(), t.clone())), Arg::N(2)]);
                or(and(s.clone(), not(p.clone())), or(two, three))
            }
            // Each region's last byte, from its fill (`Regions::body`): the closer's last, or
            // the byte before an `outside` closer; with `else E bytes M`, `E`'s `M`th byte.
            TokenRule::Read(regions) => read_last(regions, true),
            // The opener, one byte; the closer and the parts are rows of their own (`Parts::rows`).
            TokenRule::ReadParts(..) => s.clone(),
            TokenRule::Inside(last, ..) => last.clone(),
        }
    }

    /// A `read`'s regions with `open K` (`Region::open`): each `K`, and the byte where a token of the rule ends open
    /// at the file's end: its region's fill at the file's last byte, which no region's closer ends at. There the
    /// token is a `K`, as the lean lexer's `Bad`.
    pub fn open_ends(&self) -> Vec<(String, Expr)> {
        let TokenRule::Read(regions) = self else { return Vec::new() };
        let and = |a: Expr, b: Expr| Expr::And(Box::new(a), Box::new(b));
        let not = |a: Expr| Expr::Not(Box::new(a));
        let last_byte = not(Expr::Call("next".into(), vec![Arg::E(not(Expr::Empty))]));
        let closed = read_last(regions, false);
        regions.iter().filter_map(|r| r.open.as_ref().map(|k| (k.clone(), and(and(Expr::Name(format!("f_{}", r.kind)), last_byte.clone()), not(closed.clone()))))).collect()
    }
}

/// A `read`'s last byte (`TokenRule::last`): each region's, from its fill (`Regions::body`); `open`: with a region's
/// open at the file's end (`open K`).
fn read_last(regions: &[Region], open: bool) -> Expr {
    let and = |a: Expr, b: Expr| Expr::And(Box::new(a), Box::new(b));
    let or = |a: Expr, b: Expr| Expr::Or(Box::new(a), Box::new(b));
    let not = |a: Expr| Expr::Not(Box::new(a));
    let call = |f: &str, args: Vec<Expr>| Expr::Call(f.to_string(), args.into_iter().map(Arg::E).collect());
    let name = |x: String| Expr::Name(x);
    let f_open = |r: &Region| name(format!("f_{}", r.kind));
    let prev = |e: Expr, k: u8| match k {
        0 => e,
        1 => call("prev", vec![e]),
        k => Expr::Call("prev".into(), vec![Arg::E(e), Arg::N(u64::from(k))]),
    };
    let mut out: Option<Expr> = None;
    for r in regions {
        let f = name(format!("f_{}", r.kind));
        let mut last = if r.outside {
            and(f.clone(), not(call("next", vec![f])))
        } else if r.or.is_some() {
            r.closer_last(and(prev(f.clone(), 1), not(f)))
        } else {
            and(prev(f.clone(), r.bytes), not(prev(f, r.bytes - 1)))
        };
        if let Some((e, m)) = &r.empty {
            last = or(last, prev(name(e.clone()), m - 1));
        }
        // `open K`: a region open at the file's end ends at its last byte.
        if open && r.open.is_some() {
            last = or(last, and(f_open(r), not(call("next", vec![not(Expr::Empty)]))));
        }
        out = Some(match out {
            Some(o) => or(o, last),
            None => last,
        });
    }
    out.unwrap_or(Expr::Empty)
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
    /// `or C2 [bytes M]`: a second closer, of `M` bytes (1 to 3). The region runs to the first
    /// byte of either; one of `C2` is `M` bytes, the others `bytes N` (a raw string's `"""`, or
    /// its template's `$`).
    pub or: Option<(Expr, u8)>,
    /// `after K …`: the region opens only where the last region closed at its second closer
    /// (`or`, a template's `$`) before its opener, of the kinds with no `after`, is one of `K …`:
    /// a template's tail goes back to the kind of string the template is in, as the lean lexer's
    /// string reader returns to its loop after the template's code. A region in the template's
    /// code, closed at its own closer, leaves it so. Such a kind has no `else` and no `outside`;
    /// the kinds it names have an `or`.
    pub after: Vec<String>,
    /// `resumes K …` in place of `after K …`: the same openers, and the region is the rest of
    /// the token whose region its template closed, as the lean lexer reads a string with its
    /// templates as one token (decision 0600). Its reader's read makes no record of its own: it
    /// ends that token's record, or, closed at its own second closer, leaves it open over the
    /// next template's code. Tier 2 reads the template's code's records inside the string's.
    pub resumes: bool,
    /// `open K`: an opener with no closer after it anywhere in the file is a token of kind `K` to
    /// the file's end, as a lexer that finds no closer makes one bad token of the rest (`Bad`).
    /// Without it such a file is refused. Not with `else`, `outside` or an `or`. `K` is the
    /// lexer's error token: no rule takes it, `any` and `balanced` neither.
    pub open: Option<String>,
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
    ///
    /// With `or C2 bytes M`, the closer is `C | C2`, and the bytes past its first are counted
    /// from it: `c = prev(f_K) & ~f_K`, and `prev(f_K) | prev(c & ~C2) | … | prev(c & ~C2, N
    /// - 1) | prev(c & C2) | … | prev(c & C2, M - 1)`. A kind with `after A …`
    /// comes after the kinds with none, which it reads: its openers are where the last of
    /// their closers at a `C2` before (`c & C2`, `t_K`) is an `A`'s, `o_K = O & ~H &
    /// prev(fill(t_A | …, t_B | …))`, the `B`s the other kinds with an `or`. With a kind that
    /// `resumes`, a second carry `H_ended`: the places after its closes at its own closer
    /// (`prev(c & ~C2)`, the string ended), from the round before, among the `B`s and no opener.
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
        let ended = format!("{}_ended", self.hidden);
        let resumes = self.kinds.iter().any(|r| r.resumes);
        if resumes {
            out.push(Stmt::Carry(ended.clone(), Expr::Empty));
        }
        let mut ends: Option<Expr> = None;
        let mut hide: Option<Expr> = None;
        let mut or = |e: Expr| hide = Some(match hide.take() {
            Some(h) => Expr::Or(Box::new(h), Box::new(e)),
            None => e,
        });
        let plain: Vec<&Region> = self.kinds.iter().filter(|r| r.after.is_empty() && r.or.is_some()).collect();
        // Each plain kind's closers at its second closer: where a template opens.
        let mut templates: Vec<(String, Expr)> = Vec::new();
        // Where two kinds open a byte the walk opens the first listed that may: each kind's
        // opener excludes the earlier kinds' taken.
        let mut earlier: Option<Expr> = None;
        let any = |rs: Vec<&Region>, templates: &[(String, Expr)]| rs.iter().fold(None, |a: Option<Expr>, r| {
            let o = templates.iter().find(|t| t.0 == r.kind).map(|t| t.1.clone()).expect("verified: a kind before");
            Some(match a {
                Some(a) => Expr::Or(Box::new(a), Box::new(o)),
                None => o,
            })
        });
        for r in &self.kinds {
            let mut open = and(r.opener.clone(), not(name(&self.hidden)));
            if !r.after.is_empty() {
                let named = any(plain.iter().copied().filter(|k| r.after.contains(&k.kind)).collect(), &templates).expect("verified: a kind named");
                let others = any(plain.iter().copied().filter(|k| !r.after.contains(&k.kind)).collect(), &templates).unwrap_or(Expr::Empty);
                let others = if resumes { Expr::Or(Box::new(others), Box::new(name(&ended))) } else { others };
                open = and(open, call("prev", vec![Arg::E(call("fill", vec![Arg::E(named), Arg::E(others)]))]));
                if resumes {
                    open = and(open, not(name(&ended)));
                }
            }
            if let Some(e) = &earlier {
                open = and(open, not(e.clone()));
            }
            let taken = format!("taken_{}", r.kind);
            out.push(Stmt::Mask(taken.clone(), open));
            let open = name(&taken);
            earlier = Some(match earlier.take() {
                Some(e) => Expr::Or(Box::new(e), Box::new(open.clone())),
                None => open.clone(),
            });
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
            out.push(Stmt::Mask(f.clone(), call("fill", vec![Arg::E(name(&o)), Arg::E(r.closers())])));
            if r.outside {
                or(and(prev(&f, 1), not(r.closer.clone())));
            } else if let Some((c2, m)) = &r.or {
                let pk = |e: Expr, k: u8| call("prev", vec![Arg::E(e), Arg::N(u64::from(k))]);
                let c = and(prev(&f, 1), not(name(&f)));
                let (l, l2) = (and(c.clone(), not(c2.clone())), and(c, c2.clone()));
                if r.after.is_empty() {
                    templates.push((r.kind.clone(), l2.clone()));
                }
                if r.resumes {
                    ends = Some(match ends.take() {
                        Some(e) => Expr::Or(Box::new(e), Box::new(l.clone())),
                        None => l.clone(),
                    });
                }
                or(prev(&f, 1));
                for k in 1..r.bytes {
                    or(pk(l.clone(), k));
                }
                for k in 1..*m {
                    or(pk(l2.clone(), k));
                }
            } else {
                for k in 1..=r.bytes {
                    or(prev(&f, k));
                }
            }
        }
        out.push(Stmt::Next(self.hidden.clone(), hide.unwrap_or(Expr::Empty)));
        if resumes {
            out.push(Stmt::Next(ended, call("prev", vec![Arg::E(ends.unwrap_or(Expr::Empty))])));
        }
        out
    }
}

impl Region {
    /// The name of its `else`'s mask: whether a byte of the closer comes after a byte.
    pub fn later(&self) -> String {
        format!("{}_later", self.kind)
    }

    /// Its closers: `C`, or `C | C2` with `or C2`.
    pub fn closers(&self) -> Expr {
        match &self.or {
            Some((c2, _)) => Expr::Or(Box::new(self.closer.clone()), Box::new(c2.clone())),
            None => self.closer.clone(),
        }
    }

    /// The last bytes of its closers from their first bytes `c` (a mask at their first
    /// bytes): `N - 1` bytes on, or `M - 1` where `C2` holds (`or C2 bytes M`).
    pub fn closer_last(&self, c: Expr) -> Expr {
        let and = |a: Expr, b: Expr| Expr::And(Box::new(a), Box::new(b));
        let not = |a: Expr| Expr::Not(Box::new(a));
        match &self.or {
            Some((c2, m)) => Expr::Or(Box::new(prev(and(c.clone(), not(c2.clone())), self.bytes - 1)), Box::new(prev(and(c, c2.clone()), m - 1))),
            None => prev(c, self.bytes - 1),
        }
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
    /// `leaf K R`: one leaf of kind `K` over the tokens rule `R` reads, as the lean parser reads a JSON object's braces
    /// and checks its text. `R` and the rules it reaches are choices of tokens and rules, and make no node
    /// (`desugar_leaves`).
    Leaf { kind: String, rule: String },
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
    /// `token K = start S [through T | last L | run R | pair P]`: a token at each bit of `S`,
    /// running through `T`'s bytes, or ending at the first bit of `L` at or after its start (a
    /// tier-1 mask: no fill, docs/final-shape/decisions/0071). `run R`: its first byte, then the
    /// bytes of `R` after it in a row; `pair P`: one byte, two where `P` holds at its start. Both
    /// are their `last` mask too (`TokenRule::last`), and a CPU ends the token at its start by
    /// the rule (decision 0177), with no last mask.
    Token { kind: String, start: Expr, through: Option<Expr>, last: Option<Expr>, rule: Option<TokenRule> },
    Keywords { kind: String, words: Vec<(Vec<u8>, Option<String>)>, soft: Vec<Vec<u8>>, required: bool },
    Skip(Expr),
    /// `prefix B …`: a file that starts with these bytes (one to eight) is read from after them,
    /// as a lexer strips a byte order mark before its loop (decision 0335). Its bytes are outside
    /// the stream: no class, `~` or token has them, the stream's first byte is the one after
    /// them, and positions stay the file's.
    Prefix(Vec<u8>),
    Error(String, Expr),
    /// `error "WHY" by tokens when E`: an error whose bytes are the ones the token loop refuses
    /// at (decision 0177): a first byte no token begins with, a reader's region open at the
    /// file's end or with no opener there. On the CPU the loop refuses them; elsewhere `E` is a
    /// mask, as an error's. `escape "WHY" by tokens when E` too, its reason `escape: WHY`.
    TokenError(String, Expr),
    /// `escape "WHY" when E`: a construct the lexer does not handle; a file with a bit of `E` goes
    /// to tree-sitter. Unlike an error, it does not say the file is malformed.
    Escape(String, Expr),
    /// `assume "WHY" at starts NAME = E` (decision 0607): a fact the grammar's writer knows of the
    /// token loop, that mask `NAME` is `E` at every byte a token starts at. It changes no mask: a
    /// lexer that reads tokens one at a time reads `NAME` there as `E`. The compiler proves it
    /// where it can (the prover, its answers cached by content, 0414); else a test build checks it
    /// at every token.
    ///
    /// `assume "WHY" in KIND NAME = E`: in every token of the kind, after its start through the byte
    /// its run ends at, mask `NAME` reads as `E`: the token's rule is read with `E` for it.
    Assume { why: String, scope: Option<String>, name: String, expr: Expr },
    /// `assume "WHY" at ends NAME`: mask `NAME` holds only at the last byte of a token that
    /// reads a region, so the token loop knows it there, from the region it read (a template's
    /// `{`, after its string's `${`), and a fill from the byte after it (`fill(prev(NAME), E)`) is
    /// the loop's to keep, from that token to the next byte of `E`.
    AssumeEnds { why: String, name: String },
    /// `escape "WHY" at KIND when count(E) at least N` (or `run(E)`): a test of each token of
    /// the kind over its own bytes, as a lexer checks a token it reads (decision 0177): `count`,
    /// `E`'s bits in the token at least `N`; `run`, `E` at each of its first `N` bytes (the token
    /// that long at least). A token that passes it sends the file to the fallback.
    TokenEscape { why: String, kind: String, test: TokenTest, expr: Expr, least: u32 },
    /// `tree NAME { … }`: the statements that say where each node starts and ends
    /// (docs/parser-generators/13-tape-on-the-gpu.md).
    Tree { name: String, body: Vec<Stmt> },
    /// `node KIND [nested] = G`: a node of the kind spans what `G` matches; the first is the
    /// root. `nested`: it may contain itself, and runs on a stack frame (doc 18 §1).
    /// `whole`: the root spans the whole file, from byte 0, and may hold no token. `whole
    /// padded`: from its first token or comment (none: the file's end), as tree-sitter's root
    /// starts past the file's leading blanks, its padding.
    Node { kind: String, nested: bool, whole: bool, padded: bool, body: Gram },
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
    /// has it too, after the last token. `unless after W …`: a mark whose grammar token before
    /// (the extras passed over) is one of these words is none, as lean's parser asks whether a
    /// `,` ends the line before (decision 0177). `, else escape "WHY"`: a bit with no grammar
    /// token at or after it (past the last one's start) sends the file to the fallback.
    ///
    /// `, at KIND when TEST` and `, after KIND when TEST` (`MarkCheck`): a token of the kind
    /// that passes the test (a token escape's, `TokenEscape`) is a bit of the mark at its first
    /// byte (`at`) or at the byte after its last (`after`), as a lexer marks what it reads.
    ///
    /// `, settles`: where the grammar takes it, the extras before its bit (since the grammar token
    /// before) stay in the nodes its arm closes, and those after it go after them, as
    /// tree-sitter-kotlin's scanner makes a line break the statement's semicolon: a comment on a
    /// statement's line is the statement's.
    ///
    /// `, ends`: a hidden mark that a level which may end where it holds, and does not take it,
    /// ends at, for its caller to take: tree-sitter's scanner gives its semicolon wherever one is
    /// valid, before the token after it, which the level might read on (`a` then `b` on the next
    /// line is two statements, not an infix call).
    ///
    /// `, before A B …` and `, unless before A B …` (decision 0416): the tokens from the one the
    /// mark is before, as a token mark's run (`tokens`), as a parser asks of the tokens it peeks
    /// at. `next` holds them, each with whether it is an `unless`: the mark holds where no
    /// `unless before` does and, if it has a `before`, one of them does.
    ///
    /// `, drops [unless] after A … before B …` (with `settles`; `GapPattern`): where its arm runs
    /// at a token where the question holds, the extra pending last is dropped, as the lean
    /// lexer's `asi` loses the comment it swallows into the semicolon (decision 0416).
    ///
    /// `, loses [unless] after A … before B …`: where a level asks for the mark at a token where
    /// the question holds, the extra pending last is of no width, at the token's start: the lean
    /// lexer's `asi` reads a block comment that starts the line through, finds that the line goes
    /// on, and tree-sitter loses the comment's bytes (its `lost`; decision 0451). It holds only
    /// where the mark does not. Each clause is one entry; a mark's entries together decide.
    ///
    /// `, refuses [unless] after A … [before B …]`: where a level asks for the mark at a token
    /// where the question holds, the file is refused, as where the lean lexer's `asi` reads what
    /// the grammar does not say yet (decision 0451). `, refuses taken …`: only where a level takes
    /// the mark (it holds there), as the lean lexer's `asi` reads on in a constructor's context.
    ///
    /// `, passes M`: taken, it passes mark `M` too, though `M` comes after it: a mark tested
    /// before `M` where both hold (an annotation's line break before `semi`), which leaves no
    /// `M` at the token, as a line break that is no semicolon leaves none.
    Mark { kind: String, at: Expr, end: bool, unless: Vec<Vec<u8>>, unplaced: Option<String>, checks: Vec<MarkCheck>, settles: bool, ends: bool, next: Vec<(bool, Vec<Vec<TokenElem>>)>, drops: Vec<GapPattern>, loses: Vec<GapPattern>, refuses: Vec<GapPattern>, refuses_taken: Vec<GapPattern>, passes: Vec<String> },
    /// `mark KIND = tokens [after E] E …` (decision 0129): a mark before a token that begins a
    /// run of tokens, each one of an element's choices (`|`): a token kind (a name) or a word
    /// (a string), the extras between them skipped. `after`: the token before is one of
    /// `before`'s, or (`after ~E`) none of them, or no token at all. The mark is at the first
    /// element's token. `, unless before E …`: none where one of these runs is the tokens from
    /// the mark's on, as a mark's (`Mark::next`, the unless ones).
    TokenMark { kind: String, before: Vec<TokenElem>, seq: Vec<Vec<TokenElem>>, next: Vec<(bool, Vec<Vec<TokenElem>>)> },
    /// `hidden KIND …` (in a tree): marks taken where the grammar can take them and skipped
    /// elsewhere. A mark not hidden, where the grammar can not take it, refuses the file.
    Hidden(Vec<String>),
    /// `untaken KIND …` (in a tree): tokens no rule takes, the lexer's errors, as a region's `open K`
    /// is: `any` and `balanced` take them no more than a rule does, as the lean parser fails at a
    /// `Bad` where it reads any token.
    Untaken(Vec<String>),
    /// `refuse MARK in LEVEL …` (in a tree): a hidden mark that refuses the file between a
    /// nested level's first token and its last (`json_bad` in a JSON document's levels).
    Refuse { mark: String, levels: Vec<String> },
    /// `reading NAME = ITEM, …` (in a tree): a way to read a made node (`reread`).
    Reading { name: String, reading: Reading },
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
    /// `recover NODE at P, past P, …` (in a tree): `NODE`'s repetition is a file's statement loop, as a
    /// hand-written parser's. A statement that fails is cut back to its first token and made one ERROR
    /// leaf, from that token to the resume point: the first token after it where an `at` pattern holds,
    /// or past the first where a `past` pattern holds (decision 0650).
    Recover { node: String, clauses: Vec<RecoverClause> },
    /// `ending RULE K as M [inner G]`, `ending RULE K refused` (in a tree): where nested level `RULE` ends, the node
    /// made last at it, where it is a `K`, becomes an `M`, with no field of its own (its call's, where it returns, as
    /// `decide`'s), the node its wrap holds first taking field `G`; or the file is refused (`refused`). The lean
    /// parser's question after a call: `literal`'s `retag_atom` where its `term` was a call alone, a negation's
    /// failure where its operand was a comparison (decision 0668).
    Ending { rule: String, kind: String, ending: Ending },
}

/// What `ending` does with the node made last where its level ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ending {
    /// `as M [inner G]`: the node becomes an `M`, its field its call's, and the node its wrap holds first takes `G`.
    Relabel { moved: String, inner: Option<String> },
    /// `refused`: the file is refused (a statement, where the grammar recovers).
    Refused,
}

/// One of `recover`'s patterns: `at` (the resume point at its first token) or `past` (past it), its
/// tokens (each a choice, as a token mark's), and `first`: its first token first on its line (only
/// blanks before it since a line terminator of the language).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoverClause {
    pub past: bool,
    pub first: bool,
    pub seq: Vec<Vec<TokenElem>>,
}

/// The kind `implied` gives a token (`A as IMPLIED`): it makes no leaf.
pub const IMPLIED: &str = "\u{0}implied";
/// An `implied` token written as its kind (`implied R K`), not its text: this before the kind.
const IMPLIED_KIND: &str = "\u{1}";
/// The kind `drop` gives the node just made at this level (`relabel _ as GONE`): it goes.
pub const GONE: &str = "\u{0}gone";
/// The field `decide` gives the node just made at this level: none of its own, the one its call sets where it
/// returns.
pub const DECIDED: &str = "\u{0}decided";

/// Each `implied RULE 'A' … K …` of a tree's body, applied: its tokens in `RULE`'s own body
/// marked `A as IMPLIED`, each written `'A'` or a token kind `K` (a character's content, whose
/// bytes the character's node spans). A token not written there is an error.
fn apply_implied(body: &mut [Stmt]) -> Result<(), String> {
    fn mark(g: &mut Gram, tokens: &[String], found: &mut Vec<bool>) {
        match g {
            Gram::Lit(t) => {
                if let Some(i) = tokens.iter().position(|x| x == t) {
                    found[i] = true;
                    *g = Gram::As(IMPLIED.into(), Box::new(Gram::Lit(t.clone())));
                }
            }
            Gram::Sym(k) => {
                if let Some(i) = tokens.iter().position(|x| x.strip_prefix(IMPLIED_KIND) == Some(k.as_str())) {
                    found[i] = true;
                    *g = Gram::As(IMPLIED.into(), Box::new(Gram::Sym(k.clone())));
                }
            }
            // A token given a kind of its own, or merged, is a node's: not implied.
            Gram::As(..) | Gram::Merge(..) | Gram::Balanced { .. } | Gram::Any(_) | Gram::Leaf { .. } => {}
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
            return Err(format!("implied {rule}: {:?} is not written in it", tokens[i].trim_start_matches(IMPLIED_KIND)));
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
    /// The regions read so far (`regions`), for a token's `read` (`TokenRule::Read`).
    regions: Vec<Region>,
    /// Statements the one just read needs before it: a `pairs`' classes, one a byte, each made
    /// once (`classed`).
    before: Vec<Stmt>,
    /// Statements the one just read makes after it: a region's parts' tokens (`Parts::rows`).
    after: Vec<Stmt>,
    classed: Vec<u8>,
    /// The `pairs` read so far, by name, for a token's `pair` (`TokenRule::Pairs`).
    pair_sets: Vec<(String, Vec<Vec<u8>>, bool)>,
}

type R<T> = Result<T, String>;

/// For the IR's tests of its backward pass (decision 0416): the masks and numbers named in
/// `names`, each written `fill(…)`, made `back_fill(…)`, as a region's `else` makes one
/// (`Regions::body`). A grammar can not write `back_fill`; the IR keeps the operation.
pub fn backward(mut stmts: Vec<Stmt>, names: &[&str]) -> Result<Vec<Stmt>, String> {
    fn made(stmts: &mut [Stmt], name: &str) -> bool {
        stmts.iter_mut().any(|s| match s {
            Stmt::Mask(n, Expr::Call(f, _)) if n == name && f == "fill" => {
                *f = "back_fill".into();
                true
            }
            Stmt::Number(n, NExpr::Fill { back, .. }) if n == name => {
                *back = true;
                true
            }
            Stmt::Rounds { body, .. } => made(body, name),
            _ => false,
        })
    }
    for name in names {
        if !made(&mut stmts, name) {
            return Err(format!("no `fill` named `{name}`"));
        }
    }
    Ok(stmts)
}

/// Why `back_fill` is refused, and what to write instead (decision 0416).
pub const BACK_FILL_REMOVED: &str = "`back_fill` is removed from Bitlex (decision 0416). A value at a byte that is made from the \
bytes after it needs a pass backward over the whole file, and the lean parsers never make one: they look ahead from \
where they stand, and only where they need to. Write what the back fill asks, at the token:
  - the next token, or a run of tokens from here: a token mark, `mark K = tokens [after E] A B ...`, \
which tier 2 tests at the token, as a parser peeks; a run of choices is `(A | B)*`;
  - a gap mark that the token after the gap decides: `mark K = E, before A | B ...` or \
`mark K = E, unless before A | B ...`, which tier 2 tests at that token, as a scanner asks of the next token;
  - the token's own later bytes: a test of the token, `escape \"WHY\" at KIND when count(E) at least N`, \
or a mark's `at KIND when ...`;
  - what the text is inside (a class's header, a declaration's run up to its `(`): the tree's rules, \
which carry it as a parser carries its state.
The IR keeps its backward pass: a region's `else` makes one, and the IR's tests reach it through \
`syntax::backward`.";

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
        // `(C | …)*`: a run of them (`Many`), alone in its element.
        if self.eat_p("(") {
            let inner = self.token_choices()?;
            // `with M`, `without M`: each token of the run with or without mark `M`, as the lean parser's
            // `on_line` asks of each word it reads.
            let inner = if matches!(inner.as_slice(), [TokenElem::Not(_)]) { inner } else { self.with_marks(inner)? };
            self.want_p(")")?;
            self.want_p("*")?;
            // Kinds and words, or one `~` of them: a run of any other tokens.
            let not = matches!(inner.as_slice(), [TokenElem::Not(_)]);
            if !not && inner.iter().any(|c| !matches!(c, TokenElem::Kind(_) | TokenElem::Word(_) | TokenElem::With(_) | TokenElem::Without(_))) {
                return self.err("a run of kinds and words (`with`, `without` a mark), or of a `~` of them");
            }
            // `nest (O to C) …`: the depths it counts (`Nest`).
            let mut nests = vec![];
            if self.eat_kw("nest") {
                while self.eat_p("(") {
                    let open = self.token_choices()?;
                    if !self.eat_kw("to") {
                        return self.err("`to` between a depth's openers and its closers");
                    }
                    let close = self.token_choices()?;
                    self.want_p(")")?;
                    let plain = |c: &[TokenElem]| c.iter().all(|x| matches!(x, TokenElem::Kind(_) | TokenElem::Word(_)));
                    if !plain(&open) || !plain(&close) || open.iter().any(|o| close.contains(o)) {
                        return self.err("a depth's openers and closers: kinds and words, none both");
                    }
                    nests.push(Nest { open, close });
                }
                if nests.is_empty() || nests.len() > 4 {
                    return self.err("`nest` and one to four `(O to C)`");
                }
            }
            // `ending (C)`: the run's last token one of `C` where the rest is tried.
            let mut ending = vec![];
            if self.eat_kw("ending") {
                self.want_p("(")?;
                ending = self.token_choices()?;
                self.want_p(")")?;
                if !ending.iter().all(|x| matches!(x, TokenElem::Kind(_) | TokenElem::Word(_))) {
                    return self.err("a run's `ending` of kinds and words");
                }
            }
            return Ok(vec![TokenElem::Many(inner, nests, ending)]);
        }
        loop {
            if self.eat_p("~") {
                // `~C` or `~(C | …)`: none of the choices.
                let inner = if self.eat_p("(") {
                    let c = self.token_choices()?;
                    self.want_p(")")?;
                    c
                } else {
                    let mut c = self.token_choices_one()?;
                    std::mem::take(&mut c)
                };
                if inner.iter().any(|c| matches!(c, TokenElem::Not(_) | TokenElem::End)) || !out.is_empty() || self.is_p("|") {
                    return self.err("a `~` of kinds and words, alone in its element");
                }
                out.push(TokenElem::Not(inner));
                return Ok(out);
            }
            if self.eat_kw("end") {
                out.push(TokenElem::End);
                if !self.eat_p("|") {
                    return Ok(out);
                }
                continue;
            }
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
    /// An element's `with M` and `without M`, after its choices of kinds and words.
    fn with_marks(&mut self, mut c: Vec<TokenElem>) -> R<Vec<TokenElem>> {
        loop {
            let with = if self.eat_kw("without") {
                false
            } else if self.eat_kw("with") {
                true
            } else {
                return Ok(c);
            };
            if !c.iter().all(|x| matches!(x, TokenElem::Kind(_) | TokenElem::Word(_) | TokenElem::Without(_) | TokenElem::With(_))) {
                return self.err("`with` or `without` after an element of kinds and words");
            }
            let m = self.ident()?;
            c.push(if with { TokenElem::With(m) } else { TokenElem::Without(m) });
        }
    }
    /// A `drops` or `loses` question (`GapPattern`): `[unless] [after A …] before B …`, each
    /// element of kinds and words (or a `~` of them), its `with M`, `without M` and `starting
    /// "B"`, or `starting "B"` alone.
    fn gap_pattern(&mut self) -> R<GapPattern> {
        let unless = self.eat_kw("unless");
        let mut after = vec![];
        let since = self.eat_kw("since");
        if since || self.eat_kw("after") {
            while !self.is_kw("before") && !matches!(self.peek(), None | Some(Tok::Nl | Tok::P(","))) {
                after.push(self.gap_elem()?);
            }
            // `after` alone: a question of the records before the token only.
            if !after.is_empty() && !self.is_kw("before") {
                return Ok(GapPattern { unless, since, after, before: vec![] });
            }
        }
        self.want_kw("before")?;
        let mut before = vec![self.gap_elem()?];
        while matches!(self.peek(), Some(Tok::Str(_) | Tok::P("~"))) || matches!(self.peek(), Some(Tok::Ident(_))) && !self.is_kw("end") {
            before.push(self.gap_elem()?);
        }
        if since && after.is_empty() {
            return self.err("`since` and an element or more");
        }
        Ok(GapPattern { unless, since, after, before })
    }
    /// One element of a gap pattern (`gap_pattern`).
    fn gap_elem(&mut self) -> R<Vec<TokenElem>> {
        let mut c = if self.is_kw("starting") { vec![] } else { self.token_choices()? };
        if matches!(c.as_slice(), [TokenElem::Many(..)]) || c.contains(&TokenElem::End) {
            return self.err("a gap pattern's element of kinds and words, or a `~` of them");
        }
        loop {
            if self.eat_kw("starting") {
                let b = self.string()?;
                if b.is_empty() || c.iter().any(|x| matches!(x, TokenElem::Not(_))) {
                    return self.err("`starting` and a byte or more, not on a `~`");
                }
                c.push(TokenElem::Starting(b));
                continue;
            }
            let with = if self.eat_kw("without") {
                false
            } else if self.eat_kw("with") {
                true
            } else {
                return Ok(c);
            };
            let m = self.ident()?;
            c.push(if with { TokenElem::With(m) } else { TokenElem::Without(m) });
        }
    }
    /// A run of elements, as a token mark's (`tokens`) or a mark's `before`: each an element of
    /// choices, a `~` or a run; one run at most, never first or last; an `end` only last.
    fn token_seq(&mut self) -> R<Vec<Vec<TokenElem>>> {
        let mut seq = vec![];
        while matches!(self.peek(), Some(Tok::Ident(_) | Tok::Str(_) | Tok::P("~") | Tok::P("("))) {
            let c = self.token_choices()?;
            let c = self.with_marks(c)?;
            seq.push(c);
        }
        let run = |c: &[TokenElem]| matches!(c, [TokenElem::Many(..)]);
        let runs: Vec<usize> = (0..seq.len()).filter(|&i| run(&seq[i])).collect();
        if seq.is_empty() {
            return self.err("a token");
        }
        if runs.len() > 1 || runs.first().is_some_and(|&i| i == 0 || i + 1 == seq.len()) {
            return self.err("one run of tokens at most, after the first token and before the last");
        }
        if seq[..seq.len() - 1].iter().any(|c| c.contains(&TokenElem::End)) {
            return self.err("an `end` only last");
        }
        Ok(seq)
    }
    /// One kind or word, for a `~` of one.
    fn token_choices_one(&mut self) -> R<Vec<TokenElem>> {
        match self.peek().cloned() {
            Some(Tok::Str(s)) => {
                self.i += 1;
                Ok(vec![TokenElem::Word(s)])
            }
            Some(Tok::Ident(n)) if n != "end" => {
                self.i += 1;
                Ok(vec![TokenElem::Kind(n)])
            }
            _ => self.err("a token kind or a word"),
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
            // `decide`: the node just made at this level has no field of its own now, its call's where it returns
            // (`relabel DECIDED`), as the lean parser's caller sets the field of the node a call made; the rest of the
            // sequence is its body.
            if matches!(self.peek(), Some(Tok::Ident(w)) if w == "decide") {
                self.i += 1;
                let body = Box::new(self.gram_seq()?);
                items.push(Gram::Relabel { field: Some(DECIDED.into()), moved: None, inner: None, body });
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
        // `implied 'A'`: this one token makes no leaf, as the lean parser's `implied()` where another place of the same
        // rule writes it (`anon()`); `implied RULE 'A'` marks every one of a rule's.
        if name == "implied" && matches!(self.peek(), Some(Tok::Str(_))) {
            return Ok(Gram::As(IMPLIED.into(), Box::new(Gram::Lit(self.kind()?))));
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
            // `refused`: no way on, as the lean parser's `Err(())`: a choice with no alternative.
            "refused" => Ok(Gram::Alt(vec![])),
            "balanced" => {
                let kind = self.kind()?;
                let open = self.kind()?;
                let close = self.kind()?;
                Ok(Gram::Balanced { kind, open, close })
            }
            "leaf" => {
                let kind = self.kind()?;
                let rule = self.ident()?;
                Ok(Gram::Leaf { kind, rule })
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
        if last == "back_fill" {
            return Err(BACK_FILL_REMOVED.into());
        }
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
        if name == "back_fill" {
            return Err(BACK_FILL_REMOVED.into());
        }
        if name == "fill" && self.eat_p("(") {
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
    /// `regions H { K = O .. C [bytes N] [outside] [else E bytes M] [open K] … }` (`Regions`).
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
            let or = if self.eat_kw("or") {
                let c = self.expr()?;
                let m = if self.eat_kw("bytes") { self.int()? } else { 1 };
                if !(1..=3).contains(&m) {
                    return self.err("a second closer of 1 to 3 bytes");
                }
                Some((c, m as u8))
            } else {
                None
            };
            let outside = self.eat_kw("outside");
            let empty = if self.eat_kw("else") {
                let e = self.ident()?;
                self.want_kw("bytes")?;
                Some((e, self.int()?))
            } else {
                None
            };
            let open = if self.eat_kw("open") { Some(self.ident()?) } else { None };
            if open.is_some() && (outside || empty.is_some() || or.is_some()) {
                return self.err("`open` on a kind with no `outside`, `else` or `or`");
            }
            let mut after = Vec::new();
            let resumes = self.eat_kw("resumes");
            if resumes || self.eat_kw("after") {
                while matches!(self.peek(), Some(Tok::Ident(_))) {
                    after.push(self.ident()?);
                }
                if after.is_empty() || outside || empty.is_some() || open.is_some() {
                    return self.err("`after` or `resumes` and the kinds it names, on a kind with no `outside` or `else`");
                }
            }
            if !(1..=3).contains(&bytes) || (outside && bytes != 1) || (outside && or.is_some()) || empty.as_ref().is_some_and(|e| !(1..=3).contains(&e.1)) {
                return self.err("a closer of 1 to 3 bytes (`outside`: 1, and no `or`), an `else` of 1 to 3");
            }
            kinds.push(Region { kind, opener, closer, bytes: bytes as u8, outside, empty: empty.map(|(e, m)| (e, m as u8)), or, after, resumes, open });
            self.nls();
        }
        if kinds.is_empty() {
            return self.err("a region");
        }
        // `after` names kinds of these regions with no `after` of their own and with an `or` (a
        // template's closer); at most 15 kinds.
        for r in &kinds {
            for a in &r.after {
                if !kinds.iter().any(|k| k.kind == *a && k.after.is_empty() && k.or.is_some()) {
                    return Err(format!("region {}: `after {a}`, not a kind of these regions with an `or` and no `after`", r.kind));
                }
            }
        }
        // The kinds with `after` come last: they read the openers the plain kinds took.
        if let Some(w) = kinds.windows(2).find(|w| !w[0].after.is_empty() && w[1].after.is_empty()) {
            return Err(format!("region {}: a kind with no `after` after {}, which has one; the kinds with `after` come last", w[1].kind, w[0].kind));
        }
        if kinds.len() > 15 && kinds.iter().any(|r| !r.after.is_empty()) {
            return Err("regions with `after`: at most 15 kinds".into());
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
            let st = self.stmt()?;
            out.append(&mut self.before);
            out.push(st);
            out.append(&mut self.after);
            self.nls();
        }
        Ok(out)
    }
    /// `pairs P = "AB" … "ABC" …`: the mask of the pairs' first bytes, a run's every other from
    /// the left (`two & ~escaped(two)`, `two` each pair's first byte where its second is next), of
    /// hidden classes one a byte; with triples, `P_three` too: where `P` holds and a triple's
    /// three bytes are, its token three bytes. Each triple's first two bytes are a pair, as a
    /// lexer reads its next byte, then the one after. The list kept for a token's `pair P`
    /// (`TokenRule::Pairs`).
    fn pairs(&mut self, name_of: String) -> R<Stmt> {
        let mut pairs: Vec<Vec<u8>> = vec![];
        while let Some(Tok::Str(_)) = self.peek() {
            let w = self.string()?;
            if w.len() != 2 && w.len() != 3 {
                return self.err("a pair of two bytes, or a triple of three");
            }
            pairs.push(w);
        }
        if pairs.iter().any(|t| t.len() == 3 && !pairs.iter().any(|p| p[..] == t[..2])) {
            return self.err("a triple whose first two bytes are no pair");
        }
        let name = |b: u8| format!("pair_byte_{b:02x}");
        for &b in pairs.iter().flatten() {
            if !self.classed.contains(&b) {
                self.classed.push(b);
                self.before.push(Stmt::Class(name(b), vec![(u32::from(b), u32::from(b))]));
            }
        }
        let and = |a: Expr, b: Expr| Expr::And(Box::new(a), Box::new(b));
        let next = |e: Expr| Expr::Call("next".into(), vec![Arg::E(e)]);
        let or = |x: Expr, y: Expr| Expr::Or(Box::new(x), Box::new(y));
        let two = pairs.iter().filter(|p| p.len() == 2).map(|p| and(Expr::Name(name(p[0])), next(Expr::Name(name(p[1]))))).reduce(or).ok_or("a pair")?;
        let two_of = two.clone();
        let mask = and(two.clone(), Expr::Not(Box::new(Expr::Call("escaped".into(), vec![Arg::E(two)]))));
        let next2 = |e: Expr| Expr::Call("next".into(), vec![Arg::E(e), Arg::N(2)]);
        let three = pairs.iter().filter(|p| p.len() == 3).map(|p| and(and(Expr::Name(name(p[0])), next(Expr::Name(name(p[1])))), next2(Expr::Name(name(p[2]))))).reduce(or);
        let has_three = three.is_some();
        if let Some(t) = three {
            self.after.push(Stmt::Mask(format!("{name_of}_three"), and(t, Expr::Name(name_of.clone()))));
            self.after.push(Stmt::Mask(format!("{name_of}_two"), two_of));
        }
        self.pair_sets.push((name_of.clone(), pairs, has_three));
        Ok(Stmt::Mask(name_of, mask))
    }
    /// `munch K over O { "L" [-> KIND] … }`: operator tokens by longest match over each run of
    /// `O` (03-bitlex.md §3.3), as masks and tokens. At each byte the longest literal that
    /// starts there (1 to 3 bytes, all of them in `O`); a byte starts a token unless the token
    /// one or two bytes before covers it, a fixed point taken four times from "every byte". A
    /// run that has not settled by then (a chain of candidates such as `=====`) is an error, as
    /// is a byte of `O` no literal starts. A token's kind is its alias, else its literal.
    fn munch(&mut self, kind: &str, over: Expr, lits: &[(Vec<u8>, Option<String>)]) -> R<Stmt> {
        let and = |a: Expr, b: Expr| Expr::And(Box::new(a), Box::new(b));
        let or = |a: Expr, b: Expr| Expr::Or(Box::new(a), Box::new(b));
        let not = |a: Expr| Expr::Not(Box::new(a));
        let shift = |f: &str, e: Expr, k: usize| match k {
            0 => e,
            1 => Expr::Call(f.into(), vec![Arg::E(e)]),
            k => Expr::Call(f.into(), vec![Arg::E(e), Arg::N(k as u64)]),
        };
        let any = |v: Vec<Expr>| v.into_iter().reduce(|a, b| or(a, b));
        let n = |x: &str| Expr::Name(format!("munch_{kind}_{x}"));
        if lits.iter().any(|(l, _)| l.is_empty() || l.len() > 3) {
            return self.err("munch: literals of 1 to 3 bytes");
        }
        let class = |b: u8| format!("pair_byte_{b:02x}");
        for &b in lits.iter().flat_map(|(l, _)| l) {
            if !self.classed.contains(&b) {
                self.classed.push(b);
                self.before.push(Stmt::Class(class(b), vec![(u32::from(b), u32::from(b))]));
            }
        }
        let mut out = vec![Stmt::Mask(format!("munch_{kind}_over"), over)];
        // Each literal where it starts, and the bytes the longest of each length starts at.
        let mut by_len: [Vec<Expr>; 3] = Default::default();
        for (j, (l, _)) in lits.iter().enumerate() {
            let at = l.iter().enumerate().map(|(k, &b)| and(shift("next", Expr::Name(class(b)), k), shift("next", n("over"), k))).reduce(|a, b| and(a, b)).expect("a byte");
            out.push(Stmt::Mask(format!("munch_{kind}_at{j}"), at));
            by_len[l.len() - 1].push(n(&format!("at{j}")));
        }
        let zero = || Expr::Name("empty".into());
        let [one, two, three] = by_len.map(|v| any(v).unwrap_or_else(zero));
        out.push(Stmt::Mask(format!("munch_{kind}_l3"), three));
        out.push(Stmt::Mask(format!("munch_{kind}_l2"), and(two, not(n("l3")))));
        out.push(Stmt::Mask(format!("munch_{kind}_l1"), and(one, not(or(n("l2"), n("l3"))))));
        // The fixed point: a byte starts a token unless one started a byte or two before covers it.
        out.push(Stmt::Mask(format!("munch_{kind}_win0"), n("over")));
        for k in 1..=4 {
            let w = n(&format!("win{}", k - 1));
            let covered = or(shift("prev", and(w.clone(), or(n("l2"), n("l3"))), 1), shift("prev", and(w, n("l3")), 2));
            out.push(Stmt::Mask(format!("munch_{kind}_win{k}"), and(n("over"), not(covered))));
        }
        out.push(Stmt::Error(format!("munch {kind}: a chain of operators longer than its bound"), Expr::Xor(Box::new(n("win3")), Box::new(n("win4")))));
        out.push(Stmt::Error(format!("munch {kind}: a byte no operator starts"), and(n("win3"), not(or(n("l1"), or(n("l2"), n("l3")))))));
        let last = or(or(and(n("win3"), n("l1")), shift("prev", and(n("win3"), n("l2")), 1)), shift("prev", and(n("win3"), n("l3")), 2));
        out.push(Stmt::Mask(format!("munch_{kind}_last"), last));
        // A token a kind: the bytes where the longest literal is one of the kind's.
        let mut kinds: Vec<(String, Vec<Expr>)> = Vec::new();
        for (j, (l, alias)) in lits.iter().enumerate() {
            let k = alias.clone().unwrap_or_else(|| String::from_utf8_lossy(l).into_owned());
            let longest = and(n(&format!("at{j}")), n(&format!("l{}", l.len())));
            match kinds.iter_mut().find(|(x, _)| *x == k) {
                Some((_, v)) => v.push(longest),
                None => kinds.push((k, vec![longest])),
            }
        }
        for (k, v) in kinds {
            let start = and(n("win3"), any(v).expect("a literal"));
            out.push(Stmt::Token { kind: k, start, through: None, last: Some(n("last")), rule: None });
        }
        let last = out.pop().expect("a token");
        self.before.extend(out);
        Ok(last)
    }

    /// A token test: `count(E)`, `run(E)` or `all(E)`, `at least N` (`TokenTest`).
    fn token_test(&mut self) -> R<(TokenTest, Expr, u32)> {
        let test = if self.eat_kw("count") {
            TokenTest::Count
        } else if self.eat_kw("run") {
            TokenTest::Run
        } else if self.eat_kw("all") {
            TokenTest::All
        } else {
            return self.err("`count`, `run` or `all`");
        };
        self.want_p("(")?;
        let expr = self.expr()?;
        self.want_p(")")?;
        self.want_kw("at")?;
        self.want_kw("least")?;
        let least = u32::try_from(self.int()?).map_err(|_| "a token test's count past u32".to_string())?;
        if least == 0 || (test == TokenTest::Run && least > 64) {
            return self.err("a count from 1 (a run's at most 64)");
        }
        Ok((test, expr, least))
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
            // The deprecation period of `back_fill` is over (decision 0416).
            "deprecated" => return Err(BACK_FILL_REMOVED.into()),
            "lexer" => Stmt::Lexer(self.ident()?),
            "class" => {
                let n = self.ident()?;
                self.want_p("=")?;
                Stmt::Class(n, self.class_items()?)
            }
            "pairs" => {
                let n = self.ident()?;
                self.want_p("=")?;
                self.pairs(n)?
            }
            "assume" => {
                let why = String::from_utf8_lossy(&self.string()?).into_owned();
                let scope = if self.eat_kw("in") {
                    Some(self.ident()?)
                } else {
                    self.want_kw("at")?;
                    if self.eat_kw("ends") {
                        return Ok(Stmt::AssumeEnds { why, name: self.ident()? });
                    }
                    self.want_kw("starts")?;
                    None
                };
                let name = self.ident()?;
                self.want_p("=")?;
                Stmt::Assume { why, scope, name, expr: self.expr()? }
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
                self.regions.extend(regions.kinds.iter().cloned());
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
                let by_tokens = self.eat_kw("by");
                if by_tokens {
                    self.want_kw("tokens")?;
                }
                self.want_kw("when")?;
                if by_tokens { Stmt::TokenError(m, self.expr()?) } else { Stmt::Error(m, self.expr()?) }
            }
            "skip" => Stmt::Skip(self.expr()?),
            "prefix" => {
                let items = self.class_items()?;
                if items.len() > 8 || items.iter().any(|&(lo, hi)| lo != hi || hi > 0xff) {
                    return self.err("a prefix of one to eight single bytes");
                }
                Stmt::Prefix(items.iter().map(|&(b, _)| b as u8).collect())
            }
            "escape" => {
                let m = String::from_utf8_lossy(&self.string()?).into_owned();
                // `escape "WHY" by tokens when E`: the token loop's, as an error's
                // (`Stmt::TokenError`), its reason an escape's.
                if self.eat_kw("by") {
                    self.want_kw("tokens")?;
                    self.want_kw("when")?;
                    Stmt::TokenError(format!("escape: {m}"), self.expr()?)
                } else if self.eat_kw("at") {
                    let kind = self.ident()?;
                    self.want_kw("when")?;
                    let (test, expr, least) = self.token_test()?;
                    Stmt::TokenEscape { why: m, kind, test, expr, least }
                } else {
                    self.want_kw("when")?;
                    Stmt::Escape(m, self.expr()?)
                }
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
                loop {
                    match self.peek() {
                        Some(Tok::Str(s)) => tokens.push(String::from_utf8_lossy(s).into_owned()),
                        Some(Tok::Ident(k)) => tokens.push(format!("{IMPLIED_KIND}{k}")),
                        _ => break,
                    }
                    self.i += 1;
                }
                if tokens.is_empty() {
                    return self.err("implied RULE 'A' … K …");
                }
                Stmt::Implied { rule, tokens }
            }
            "node" => {
                let kind = self.kind()?;
                let nested = self.eat_kw("nested");
                let whole = self.eat_kw("whole");
                let padded = whole && self.eat_kw("padded");
                self.want_p("=")?;
                Stmt::Node { kind, nested, whole, padded, body: self.gram()? }
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
            "ending" => {
                let rule = self.ident()?;
                let kind = self.kind()?;
                let ending = if self.eat_kw("refused") {
                    Ending::Refused
                } else if self.eat_kw("as") {
                    let moved = self.kind()?;
                    let inner = if self.eat_kw("inner") { Some(self.ident()?) } else { None };
                    Ending::Relabel { moved, inner }
                } else {
                    return self.err("ending RULE K as M [inner G] | ending RULE K refused");
                };
                Stmt::Ending { rule, kind, ending }
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
            "recover" => {
                let node = self.ident()?;
                let mut clauses = vec![];
                loop {
                    let past = match self.peek() {
                        Some(Tok::Ident(w)) if w == "at" => false,
                        Some(Tok::Ident(w)) if w == "past" => true,
                        _ => return self.err("recover NODE at P | past P, …"),
                    };
                    self.ident()?;
                    let first = self.eat_kw("first");
                    clauses.push(RecoverClause { past, first, seq: self.token_seq()? });
                    if !self.is_p(",") {
                        break;
                    }
                    self.want_p(",")?;
                }
                Stmt::Recover { node, clauses }
            }
            "extra" | "hidden" | "untaken" => {
                let mut kinds = vec![];
                while matches!(self.peek(), Some(Tok::Ident(_) | Tok::Str(_))) {
                    kinds.push(self.kind()?);
                }
                match kw.as_str() {
                    "extra" => Stmt::Extra(kinds),
                    "hidden" => Stmt::Hidden(kinds),
                    _ => Stmt::Untaken(kinds),
                }
            }
            "mark" => {
                let kind = self.kind()?;
                self.want_p("=")?;
                if self.eat_kw("tokens") {
                    let before = if self.eat_kw("after") {
                        let c = self.token_choices()?;
                        self.with_marks(c)?
                    } else {
                        vec![]
                    };
                    let seq = self.token_seq()?;
                    // The mark's token is a token of the choices: no `~`, no `end`; an `end` only in
                    // the last element. The one before (`after`) has no `end`; a `~` there holds
                    // where no token comes before too (the file's first).
                    let special = |c: &[TokenElem]| c.iter().any(|x| matches!(x, TokenElem::Not(_) | TokenElem::End | TokenElem::Many(..)));
                    let ends = |c: &[TokenElem]| c.contains(&TokenElem::End);
                    if special(&seq[0]) || ends(&before) || seq[..seq.len() - 1].iter().any(|c| ends(c)) {
                        return self.err("a mark's token of kinds and words, and `end` only last");
                    }
                    let mut next = Vec::new();
                    while self.eat_p(",") {
                        self.nls();
                        self.want_kw("unless")?;
                        self.want_kw("before")?;
                        let seq = self.token_seq()?;
                        if seq[0].iter().any(|c| !matches!(c, TokenElem::Kind(_) | TokenElem::Word(_))) {
                            return self.err("an `unless before`'s first token of kinds and words");
                        }
                        next.push((true, seq));
                    }
                    let s = Stmt::TokenMark { kind, before, seq, next };
                    self.end_stmt()?;
                    return Ok(s);
                }
                let at = self.expr()?;
                let mut unless = vec![];
                if self.eat_kw("unless") {
                    self.want_kw("after")?;
                    for c in self.token_choices()? {
                        match c {
                            TokenElem::Word(w) => unless.push(w),
                            _ => return self.err("`unless after` words only"),
                        }
                    }
                }
                let (mut end, mut unplaced, mut checks, mut settles, mut ends, mut drops, mut loses, mut refuses, mut refuses_taken) = (false, None, vec![], false, false, vec![], vec![], vec![], vec![]);
                let mut next: Vec<(bool, Vec<Vec<TokenElem>>)> = Vec::new();
                let mut passes = Vec::new();
                while self.eat_p(",") {
                    self.nls();
                    let not = self.eat_kw("unless");
                    if not || self.is_kw("before") {
                        self.want_kw("before")?;
                        // The token the mark is before: kinds and words, as a token mark's.
                        let seq = self.token_seq()?;
                        if seq[0].iter().any(|c| !matches!(c, TokenElem::Kind(_) | TokenElem::Word(_))) {
                            return self.err("a `before`'s first token of kinds and words");
                        }
                        next.push((not, seq));
                    } else if self.eat_kw("settles") {
                        settles = true;
                    } else if self.eat_kw("drops") {
                        drops.push(self.gap_pattern()?);
                    } else if self.eat_kw("loses") {
                        loses.push(self.gap_pattern()?);
                    } else if self.eat_kw("refuses") {
                        if self.eat_kw("taken") {
                            refuses_taken.push(self.gap_pattern()?);
                        } else {
                            refuses.push(self.gap_pattern()?);
                        }
                    } else if self.eat_kw("ends") {
                        ends = true;
                    } else if self.eat_kw("passes") {
                        passes.push(self.ident()?);
                    } else if self.eat_kw("else") {
                        self.want_kw("escape")?;
                        unplaced = Some(String::from_utf8_lossy(&self.string()?).into_owned());
                    } else if self.is_kw("at") || self.is_kw("after") {
                        let after = self.eat_kw("after");
                        if !after {
                            self.want_kw("at")?;
                        }
                        let kind = self.ident()?;
                        self.want_kw("when")?;
                        let (test, expr, least) = self.token_test()?;
                        checks.push(MarkCheck { after, kind, test, expr, least });
                    } else {
                        self.want_kw("end")?;
                        end = true;
                    }
                }
                if !drops.is_empty() && !settles {
                    return self.err("`drops` on a mark that settles");
                }
                Stmt::Mark { kind, at, end, unless, unplaced, checks, settles, ends, next, drops, loses, refuses, refuses_taken, passes }
            }
            "token" => {
                let kind = self.kind()?;
                self.want_p("=")?;
                self.want_kw("start")?;
                let start = self.expr()?;
                let through = if self.eat_kw("through") { Some(self.expr()?) } else { None };
                let mut last = if through.is_none() && self.eat_kw("last") { Some(self.expr()?) } else { None };
                let rule = if through.is_some() || last.is_some() {
                    None
                } else if self.eat_kw("run") {
                    let r = self.expr()?;
                    if self.eat_kw("once") { Some(TokenRule::RunOnce(r, self.expr()?)) } else { Some(TokenRule::Run(r)) }
                } else if self.eat_kw("pair") {
                    let e = self.expr()?;
                    match &e {
                        Expr::Name(n) => match self.pair_sets.iter().find(|(m, _, _)| m == n) {
                            Some((_, pairs, three)) => Some(TokenRule::Pairs { pairs: pairs.clone(), mask: e.clone(), three: three.then(|| Expr::Name(format!("{n}_three"))) }),
                            None => Some(TokenRule::Pair(e)),
                        },
                        _ => Some(TokenRule::Pair(e)),
                    }
                } else if self.eat_kw("read") {
                    let mut regions = vec![];
                    loop {
                        let k = self.ident()?;
                        let Some(r) = self.regions.iter().find(|r| r.kind == k) else {
                            return self.err("a region read before (`regions`)");
                        };
                        regions.push(r.clone());
                        if !self.eat_p("|") {
                            break;
                        }
                    }
                    if self.eat_kw("parts") {
                        if regions.iter().any(|r| r.outside || r.empty.is_some()) {
                            return self.err("a region read in parts with its closer its own, no `else`");
                        }
                        self.want_p("{")?;
                        let mut parts = vec![];
                        loop {
                            self.nls();
                            if self.eat_p("}") {
                                break;
                            }
                            let p = self.kind()?;
                            self.want_p("=")?;
                            let e = self.expr()?;
                            let len = if self.eat_kw("last") {
                                PartLen::Last(self.expr()?)
                            } else {
                                self.want_kw("bytes")?;
                                let n = u8::try_from(self.int()?).map_err(|_| "a part of at most 255 bytes".to_owned())?;
                                if n == 0 {
                                    return self.err("a part of one byte or more");
                                }
                                PartLen::Bytes(n)
                            };
                            parts.push((p, e, len));
                            self.nls();
                            let _ = self.eat_p(",");
                        }
                        self.want_kw("else")?;
                        let content = self.kind()?;
                        let mut closes = vec![];
                        if self.eat_kw("closes") {
                            self.want_p("{")?;
                            loop {
                                self.nls();
                                if self.eat_p("}") {
                                    break;
                                }
                                let k = self.kind()?;
                                self.want_p("=")?;
                                let e = self.expr()?;
                                self.want_kw("bytes")?;
                                let n = u8::try_from(self.int()?).map_err(|_| "a closer of at most 255 bytes".to_owned())?;
                                if n == 0 {
                                    return self.err("a closer of one byte or more");
                                }
                                closes.push((k, e, n));
                                self.nls();
                                let _ = self.eat_p(",");
                            }
                        }
                        Some(TokenRule::ReadParts(regions, Parts { parts, content, closes }))
                    } else {
                        Some(TokenRule::Read(regions))
                    }
                } else {
                    None
                };
                if let Some(r) = &rule {
                    last = Some(r.last(&start));
                }
                // A triple whose last byte starts a pair with the byte after (`!===`, `====`):
                // the mask's every other byte of a run (`P`) is then not the left-to-right
                // reading the token loop makes, a pair after the triple. No such text is a
                // language's, so it is an error, the same on every target.
                if let Some(TokenRule::Pairs { mask: Expr::Name(n), three: Some(t), .. }) = &rule {
                    let after = Expr::Call("next".into(), vec![Arg::E(Expr::Name(format!("{n}_two"))), Arg::N(2)]);
                    let at = Expr::And(Box::new(Expr::And(Box::new(start.clone()), Box::new(t.clone()))), Box::new(after));
                    self.after.push(Stmt::Error(format!("{kind}: a triple's last byte starts a pair"), at));
                }
                // A region read in parts: its other rows after it (`Parts::rows`).
                if let Some(TokenRule::ReadParts(rs, parts)) = &rule {
                    // Several regions: the opener a row in each's mode too, at its own openers
                    // (`start & o_K`), all of the one reader (`ReadParts`).
                    if rs.len() > 1 {
                        let at = |r: &Region| Expr::And(Box::new(start.clone()), Box::new(Expr::Name(format!("o_{}", r.kind))));
                        for r in &rs[1..] {
                            let s = at(r);
                            self.after.push(Stmt::Token { kind: kind.clone(), start: s.clone(), through: None, last: Some(s), rule: rule.clone() });
                        }
                        self.after.extend(parts.rows(&kind, rs, &start));
                        let s = at(&rs[0]);
                        return Ok(Stmt::Token { kind, start: s.clone(), through: None, last: Some(s), rule });
                    }
                    self.after.extend(parts.rows(&kind, rs, &start));
                }
                Stmt::Token { kind, start, through, last, rule }
            }
            "munch" => {
                let kind = self.ident()?;
                self.want_kw("over")?;
                let over = self.expr()?;
                let lits = self.words()?;
                self.munch(&kind, over, &lits)?
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

/// The prefix a program declares (`Stmt::Prefix`), if any; and an error where it has two, or a
/// construct whose meaning reads bytes or positions past the stream's cleaning: a machine,
/// `ucode`, hypotheses or a number (decision 0335). Every evaluator of the language asks this.
pub fn prefix_of(stmts: &[Stmt]) -> Result<Option<Vec<u8>>, String> {
    fn walk(stmts: &[Stmt], found: &mut Vec<Vec<u8>>, bad: &mut Option<&'static str>) {
        for s in stmts {
            match s {
                Stmt::Prefix(b) => found.push(b.clone()),
                Stmt::Machine(..) => *bad = Some("a machine"),
                Stmt::Ucode { .. } => *bad = Some("`ucode`"),
                Stmt::Hyp { .. } => *bad = Some("hypotheses"),
                Stmt::Number(..) => *bad = Some("a number"),
                Stmt::Block { body, .. } | Stmt::Rounds { body, .. } | Stmt::Tree { body, .. } => walk(body, found, bad),
                _ => {}
            }
        }
    }
    let (mut found, mut bad) = (Vec::new(), None);
    walk(stmts, &mut found, &mut bad);
    match (found.len(), bad) {
        (0, _) => Ok(None),
        (1, None) => Ok(found.pop()),
        (1, Some(what)) => Err(format!("a prefix with {what}: its meaning reads the bytes before the stream")),
        _ => Err("more than one prefix".into()),
    }
}

pub fn parse(src: &str) -> Result<Vec<Stmt>, String> {
    let mut p = P { t: lex(src)?, i: 0, regions: vec![], before: vec![], after: vec![], classed: vec![], pair_sets: vec![] };
    p.nls();
    let mut out = vec![];
    while p.peek().is_some() {
        let st = p.stmt()?;
        out.append(&mut p.before);
        out.push(st);
        out.append(&mut p.after);
        p.nls();
    }
    Ok(out)
}

/// A tree's statements with each `leaf K R` as the grammar's other elements say it, for the machine and the reference
/// (decision 0673 step 6): `R` read again with its first token a leaf of `K` and every token after it merged into that
/// leaf, a copy of each rule it reaches with its tokens merged, fields dropped. A `refuse M in L` holds in `L`'s copies
/// too. The Rust writer keeps `leaf`: the rules then read the tokens and make nothing, and the leaf is made after them.
pub fn desugar_leaves(stmts: &[Stmt]) -> Result<Vec<Stmt>, String> {
    fn walk(g: &Gram, f: &mut dyn FnMut(&Gram)) {
        f(g);
        match g {
            Gram::Seq(v) | Gram::Alt(v) => v.iter().for_each(|x| walk(x, f)),
            Gram::Opt(a) | Gram::Star(a) | Gram::Plus(a) | Gram::Field(_, a) | Gram::Retag(_, a) | Gram::Merge(_, a) | Gram::As(_, a) => walk(a, f),
            Gram::Wrap { body, .. } | Gram::Relabel { body, .. } | Gram::Fold { body, .. } | Gram::Reread { body, .. } | Gram::Flag { body, .. } | Gram::Unless { body, .. } => walk(body, f),
            Gram::Lit(_) | Gram::Sym(_) | Gram::Balanced { .. } | Gram::Any(_) | Gram::Leaf { .. } => {}
        }
    }
    let mut any = false;
    for s in stmts {
        if let Stmt::Node { body, .. } | Stmt::Choice { body, .. } = s {
            walk(body, &mut |g| any |= matches!(g, Gram::Leaf { .. }));
        }
    }
    if !any {
        return Ok(stmts.to_vec());
    }
    let rule = |n: &str| -> Option<(&Gram, bool, bool)> {
        stmts.iter().find_map(|s| match s {
            Stmt::Node { kind, nested, body, .. } if kind == n => Some((body, *nested, true)),
            Stmt::Choice { name, nested, body, .. } if name == n => Some((body, *nested, false)),
            _ => None,
        })
    };
    let name = |r: &str, k: &str, first: bool| format!("{r}\u{0}{} {k}", if first { "first" } else { "leaf" });
    // Each copy to write: the rule, the leaf's kind, whether the copy's first token opens the leaf.
    let mut todo: Vec<(String, String, bool)> = Vec::new();
    struct Cx<'b> {
        kind: &'b str,
        todo: &'b mut Vec<(String, String, bool)>,
    }
    fn copy<'s>(g: &Gram, first: bool, cx: &mut Cx<'_>, rule: &dyn Fn(&str) -> Option<(&'s Gram, bool, bool)>, name: &dyn Fn(&str, &str, bool) -> String) -> Result<Gram, String> {
        let k = cx.kind.to_owned();
        let token = |t: Gram| if first { Gram::As(k.clone(), Box::new(t)) } else { Gram::Merge(k.clone(), Box::new(t)) };
        Ok(match g {
            Gram::Lit(_) => token(g.clone()),
            Gram::Sym(n) => match rule(n) {
                Some((_, _, true)) => return Err(format!("leaf {k}: rule `{n}` makes a node")),
                Some(_) => {
                    if !cx.todo.iter().any(|t| t.0 == *n && t.2 == first) {
                        cx.todo.push((n.clone(), k.clone(), first));
                    }
                    Gram::Sym(name(n, &k, first))
                }
                None => token(g.clone()),
            },
            Gram::Seq(v) => {
                let mut out = Vec::new();
                for (i, x) in v.iter().enumerate() {
                    out.push(copy(x, first && i == 0, cx, rule, name)?);
                }
                Gram::Seq(out)
            }
            Gram::Alt(v) => Gram::Alt(v.iter().map(|x| copy(x, first, cx, rule, name)).collect::<Result<_, _>>()?),
            Gram::Opt(_) | Gram::Star(_) if first => return Err(format!("leaf {k}: its first token may be none")),
            Gram::Opt(a) => Gram::Opt(Box::new(copy(a, false, cx, rule, name)?)),
            Gram::Star(a) => Gram::Star(Box::new(copy(a, false, cx, rule, name)?)),
            Gram::Plus(a) if first => Gram::Seq(vec![copy(a, true, cx, rule, name)?, Gram::Star(Box::new(copy(a, false, cx, rule, name)?))]),
            Gram::Plus(a) => Gram::Plus(Box::new(copy(a, false, cx, rule, name)?)),
            // The leaf's kind is every token's; a field is the leaf's caller's.
            Gram::Field(_, a) | Gram::As(_, a) | Gram::Merge(_, a) => copy(a, first, cx, rule, name)?,
            Gram::Flag { flag, set, body } => Gram::Flag { flag: flag.clone(), set: *set, body: Box::new(copy(body, first, cx, rule, name)?) },
            Gram::Unless { flag, body } => Gram::Unless { flag: flag.clone(), body: Box::new(copy(body, first, cx, rule, name)?) },
            _ => return Err(format!("leaf {k}: its rules make nodes")),
        })
    }
    let mut out = Vec::new();
    for s in stmts {
        out.push(match s {
            Stmt::Node { kind, nested, whole, padded, body } => Stmt::Node { kind: kind.clone(), nested: *nested, whole: *whole, padded: *padded, body: leaves_in(body, &mut todo, &name)? },
            Stmt::Choice { name: n, supertype, nested, body } => Stmt::Choice { name: n.clone(), supertype: supertype.clone(), nested: *nested, body: leaves_in(body, &mut todo, &name)? },
            s => s.clone(),
        });
    }
    // `leaf K R` in `g`: the first copy of `R`.
    fn leaves_in(g: &Gram, todo: &mut Vec<(String, String, bool)>, name: &dyn Fn(&str, &str, bool) -> String) -> Result<Gram, String> {
        let mut d = |x: &Gram| leaves_in(x, todo, name);
        Ok(match g {
            Gram::Leaf { kind, rule } => {
                if !todo.iter().any(|t| t.0 == *rule && t.1 == *kind && t.2) {
                    todo.push((rule.clone(), kind.clone(), true));
                }
                Gram::Sym(name(rule, kind, true))
            }
            Gram::Seq(v) => Gram::Seq(v.iter().map(&mut d).collect::<Result<_, _>>()?),
            Gram::Alt(v) => Gram::Alt(v.iter().map(&mut d).collect::<Result<_, _>>()?),
            Gram::Opt(a) => Gram::Opt(Box::new(d(a)?)),
            Gram::Star(a) => Gram::Star(Box::new(d(a)?)),
            Gram::Plus(a) => Gram::Plus(Box::new(d(a)?)),
            Gram::Field(f, a) => Gram::Field(f.clone(), Box::new(d(a)?)),
            Gram::Retag(k, a) => Gram::Retag(k.clone(), Box::new(d(a)?)),
            Gram::Merge(k, a) => Gram::Merge(k.clone(), Box::new(d(a)?)),
            Gram::As(k, a) => Gram::As(k.clone(), Box::new(d(a)?)),
            Gram::Wrap { kind, field, moved, body } => Gram::Wrap { kind: kind.clone(), field: field.clone(), moved: moved.clone(), body: Box::new(d(body)?) },
            Gram::Relabel { field, moved, inner, body } => Gram::Relabel { field: field.clone(), moved: moved.clone(), inner: inner.clone(), body: Box::new(d(body)?) },
            Gram::Fold { kind, body } => Gram::Fold { kind: kind.clone(), body: Box::new(d(body)?) },
            Gram::Reread { reading, body } => Gram::Reread { reading: reading.clone(), body: Box::new(d(body)?) },
            Gram::Flag { flag, set, body } => Gram::Flag { flag: flag.clone(), set: *set, body: Box::new(d(body)?) },
            Gram::Unless { flag, body } => Gram::Unless { flag: flag.clone(), body: Box::new(d(body)?) },
            Gram::Lit(_) | Gram::Sym(_) | Gram::Balanced { .. } | Gram::Any(_) => g.clone(),
        })
    }
    let mut done = 0;
    let mut copies: Vec<(String, String)> = Vec::new();
    while done < todo.len() {
        let (r, k, first) = todo[done].clone();
        done += 1;
        let Some((body, nested, node)) = rule(&r) else { return Err(format!("leaf {k}: no rule `{r}`")) };
        if node {
            return Err(format!("leaf {k}: rule `{r}` makes a node"));
        }
        let mut cx = Cx { kind: &k, todo: &mut todo };
        let body = copy(body, first, &mut cx, &rule, &name)?;
        copies.push((r.clone(), name(&r, &k, first)));
        out.push(Stmt::Choice { name: name(&r, &k, first), supertype: None, nested, body });
    }
    // A mark refused in a rule is refused in its copies.
    for s in &mut out {
        if let Stmt::Refuse { levels, .. } = s {
            let more: Vec<String> = copies.iter().filter(|(r, _)| levels.contains(r)).map(|(_, c)| c.clone()).collect();
            levels.extend(more);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tier2_tests {
    use super::*;

    /// `back_fill` is refused, as a mask's and as a number's, with what to write instead
    /// (decision 0416), with or without the old `deprecated back_fill`; `backward` makes one for
    /// the IR's tests.
    #[test]
    fn back_fill_is_refused() {
        for body in ["mask m = back_fill(q, sp)\n", "number n = back_fill(q, sp, position)\n"] {
            let src = format!("lexer t\nclass q = '\"'\nclass sp = ' '\n{body}");
            assert_eq!(parse(&src).err().as_deref(), Some(BACK_FILL_REMOVED), "{body}");
            let deprecated = format!("lexer t\ndeprecated back_fill\nclass q = '\"'\nclass sp = ' '\n{body}");
            assert_eq!(parse(&deprecated).err().as_deref(), Some(BACK_FILL_REMOVED), "{body}");
        }
        // The IR's tests: a `fill` made a `back_fill` in the parsed statements.
        let src = "lexer t\nclass q = '\"'\nclass sp = ' '\nmask ahead = fill(q, sp)\nnumber n = fill(q, sp, position)\n";
        let stmts = backward(parse(src).expect("parses"), &["ahead", "n"]).expect("both are fills");
        assert!(matches!(&stmts[3], Stmt::Mask(_, Expr::Call(f, _)) if f == "back_fill"));
        assert!(matches!(&stmts[4], Stmt::Number(_, NExpr::Fill { back: true, .. })));
        assert!(backward(parse(src).expect("parses"), &["q"]).is_err());
    }

    /// A gap mark's `before` and `unless before` (decision 0416): the tokens from the one after
    /// it, several of each, a run among them; and a token mark's run.
    #[test]
    fn a_mark_is_before_a_token() {
        let mark = |m: &str| {
            let src = format!("lexer t\nclass lf = '\\n'\n{m}\n");
            parse(&src).map(|s| match s.into_iter().last() {
                Some(Stmt::Mark { next, end, .. }) => (next, end),
                other => panic!("{other:?}"),
            })
        };
        let word = |w: &str| TokenElem::Word(w.as_bytes().to_vec());
        let name = TokenElem::Kind("name".into());
        assert_eq!(mark("mark k = lf, before \".\" | name, end"), Ok((vec![(false, vec![vec![word("."), name.clone()]])], true)));
        assert_eq!(mark("mark k = lf, unless before \"}\""), Ok((vec![(true, vec![vec![word("}")]])], false)));
        // A run of tokens, then the ones after it; several `before`s, each its own.
        assert_eq!(
            mark("mark k = lf, unless before \"else\" ~\"->\", before \"{\" (name | \",\")* \"->\""),
            Ok((vec![(true, vec![vec![word("else")], vec![TokenElem::Not(vec![word("->")])]]), (false, vec![vec![word("{")], vec![TokenElem::Many(vec![name.clone(), word(",")], vec![], vec![])], vec![word("->")]])], false))
        );
        assert!(mark("mark k = lf, before ~\"}\"").is_err());
        assert!(mark("mark k = lf, before end").is_err());
        // A run first or last, two runs, a run of a `~`: refused.
        assert!(mark("mark k = lf, before (name)* \"->\"").is_err());
        assert!(mark("mark k = lf, before \"{\" (name)*").is_err());
        assert!(mark("mark k = lf, before \"{\" (name)* \",\" (name)* \"->\"").is_err());
        assert!(mark("mark k = lf, before \"{\" (~name | \",\")* \"->\"").is_err());
        assert_eq!(
            mark("mark k = lf, before \"{\" (~(name | \",\"))* \"->\""),
            Ok((vec![(false, vec![vec![word("{")], vec![TokenElem::Many(vec![TokenElem::Not(vec![name.clone(), word(",")])], vec![], vec![])], vec![word("->")]])], false))
        );
        assert!(parse("lexer t\nmark k = tokens \"{\" (name | \",\")* \"->\"\n").is_ok());
        // A run that counts depths (Q339): each `(O to C)` one; none, a token both, a `~`: refused.
        let angle = Nest { open: vec![word("<")], close: vec![word(">")] };
        let paren = Nest { open: vec![word("(")], close: vec![word(")")] };
        assert_eq!(
            mark("mark k = lf, before \"<\" (name | \"<\" | \">\" | \"(\" | \")\")* nest (\"<\" to \">\") (\"(\" to \")\") \">\" \"(\""),
            Ok((vec![(false, vec![vec![word("<")], vec![TokenElem::Many(vec![name.clone(), word("<"), word(">"), word("("), word(")")], vec![angle, paren.clone()], vec![])], vec![word(">")], vec![word("(")]])], false))
        );
        assert!(mark("mark k = lf, before \"<\" (name)* nest \">\"").is_err());
        assert!(mark("mark k = lf, before \"<\" (name)* nest (\"<\" to \"<\") \">\"").is_err());
        assert!(mark("mark k = lf, before \"<\" (name)* nest (~\"<\" to \">\") \">\"").is_err());
        // `ending (C)`: the run's last token (Q342).
        assert_eq!(
            mark("mark k = lf, before \"@\" (~\"{\")* nest (\"(\" to \")\") ending (name | \")\") ~\".\""),
            Ok((vec![(false, vec![vec![word("@")], vec![TokenElem::Many(vec![TokenElem::Not(vec![word("{")])], vec![paren.clone()], vec![name.clone(), word(")")])], vec![TokenElem::Not(vec![word(".")])]])], false))
        );
        assert!(mark("mark k = lf, before \"@\" (name)* ending (~name) \".\"").is_err());
        // Each token of a run without a mark (decision 0673 step 4); not after a `~`.
        assert_eq!(
            mark("mark k = lf, before \"{\" (name | \",\" without k)* \"}\""),
            Ok((vec![(false, vec![vec![word("{")], vec![TokenElem::Many(vec![name.clone(), word(","), TokenElem::Without("k".into())], vec![], vec![])], vec![word("}")]])], false))
        );
        assert!(mark("mark k = lf, before \"{\" (~name without k)* \"}\"").is_err());
    }

    /// A settling mark's `drops` (decision 0416, its questions 0451): parsed with `settles`,
    /// refused without; the records before the token, and those from it.
    #[test]
    fn a_settling_mark_drops_an_extra() {
        let mark = |m: &str| parse(&format!("lexer t\nclass lf = '\\n'\n{m}\n")).map(|s| match s.last() {
            Some(Stmt::Mark { drops, .. }) => drops.clone(),
            _ => vec![],
        });
        let kind = |k: &str| TokenElem::Kind(k.into());
        assert_eq!(
            mark("mark k = lf, settles, drops after ~\";\" c with k before starting \"ae\", drops unless after c before \"as\" x"),
            Ok(vec![
                GapPattern { unless: false, since: false, after: vec![vec![TokenElem::Not(vec![TokenElem::Word(b";".to_vec())])], vec![kind("c"), TokenElem::With("k".into())]], before: vec![vec![TokenElem::Starting(b"ae".to_vec())]] },
                GapPattern { unless: true, since: false, after: vec![vec![kind("c")]], before: vec![vec![TokenElem::Word(b"as".to_vec())], vec![kind("x")]] },
            ])
        );
        assert!(mark("mark k = lf, drops before \"a\"").is_err());
    }

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
        let src = "lexer t\nclass tick = '`'\nmask last = tick & ~next(tick)\nnumber end = fill(last, ~tick, position)\nnumber rem = end - position\nmask long = tick & at_least(rem, 3)\nnumber n = fill(long, long, rem)\nmask close = tick & at_least(rem, n) & ~long\nmask over = above(n, rem)\n";
        let stmts = backward(parse(src).expect("parses"), &["end"]).expect("a fill");
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
        let bad = "lexer t\nclass q = '\"'\nrounds until settled {\n  carry h = empty\n  number x = fill(q, h, position)\n  next h = prev(at_least(x, 1))\n}\n";
        let stmts = backward(parse(bad).expect("parses"), &["x"]).expect("a fill");
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
