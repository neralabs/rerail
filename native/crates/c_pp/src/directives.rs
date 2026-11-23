//! Mode (a): every directive of a file, as rows (decision 0327, "The relations").
//!
//! A directive is a logical line whose first preprocessing token is `#` (or `%:`). Every directive
//! is recorded, in every group: what the file says, whatever a configuration would take. Positions
//! are the file's bytes (`Span`s are read in the phase-2 text and mapped back, `Directives::file`);
//! a name's or body's spelling is read from the phase-2 text (`Directives::text`), so a `\`-newline
//! inside it is gone.
//!
//! The conditionals are matched by nesting: each `#if…` opens a chain, each `#elif…` and `#else`
//! links on, `#endif` closes it (`cond_next`), and every directive records the group it sits in
//! (`parent`: the `#if…`, `#elif…` or `#else` that opened it). A file's include guard is clang's
//! (the multiple-include optimization): `#ifndef X` or `#if !defined X` first, `#endif` last, with
//! no token outside.

use rerail_c_lean::pp::{BOL, Options, P, Phase2, PpLexer, PpTok, SPACE};

/// A byte range of the phase-2 text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    fn of(t: &PpTok) -> Self {
        Span { start: t.start, end: t.end }
    }

    fn to(self, t: &PpTok) -> Self {
        Span { start: self.start, end: t.end }
    }

    /// Whether the range is empty.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

macro_rules! kinds {
    ($($k:ident = $s:literal,)*) => {
        /// A directive's kind.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum Kind {
            /// `#` alone on its line.
            Null,
            /// A word that names no directive (`#foo`), or a token that is no word (`#!`).
            Unknown,
            /// GNU's line marker: `# 33 "file" 1 3`.
            LineMarker,
            $($k,)*
        }

        /// The directives' names, after an empty word (a switch's code 0 is no word): name
        /// `k + 1` is `KINDS[k]`'s.
        const NAMES: &[&[u8]] = &[b"", $($s,)*];
        const KINDS: &[Kind] = &[$(Kind::$k,)*];

        impl Kind {
            /// The kind's name, as a fact's column reads it.
            pub fn name(self) -> &'static str {
                match self {
                    Kind::Null => "null",
                    Kind::Unknown => "unknown",
                    Kind::LineMarker => "line_marker",
                    $(Kind::$k => unsafe { std::str::from_utf8_unchecked($s) },)*
                }
            }
        }
    };
}

kinds! {
    Define = b"define",
    Undef = b"undef",
    Include = b"include",
    IncludeNext = b"include_next",
    Import = b"import",
    Embed = b"embed",
    If = b"if",
    Ifdef = b"ifdef",
    Ifndef = b"ifndef",
    Elif = b"elif",
    Elifdef = b"elifdef",
    Elifndef = b"elifndef",
    Else = b"else",
    Endif = b"endif",
    Line = b"line",
    Error = b"error",
    Warning = b"warning",
    Pragma = b"pragma",
    Ident = b"ident",
    Sccs = b"sccs",
    Assert = b"assert",
    Unassert = b"unassert",
}

rerail_simd8::keyword_switch!(
    /// The directive names' switch (`rerail_simd8::words`): a name's code is its index in `NAMES`.
    static DIRECTIVE_WORDS = NAMES, fold: false
);

impl Kind {
    /// The kind a directive's name `word` names (`Unknown`: none).
    #[inline]
    pub fn of(word: &[u8]) -> Kind {
        match DIRECTIVE_WORDS.find(word, 0, word.len()) {
            0 => Kind::Unknown,
            c => KINDS[c as usize - 1],
        }
    }

    /// Whether the kind opens, continues or closes a conditional.
    pub fn is_cond(self) -> bool {
        matches!(self, Kind::If | Kind::Ifdef | Kind::Ifndef | Kind::Elif | Kind::Elifdef | Kind::Elifndef | Kind::Else | Kind::Endif)
    }

    /// Whether the kind's operand is a header (`<…>`, `"…"`, or tokens that make one).
    pub fn is_include(self) -> bool {
        matches!(self, Kind::Include | Kind::IncludeNext | Kind::Import | Kind::Embed)
    }
}

/// A directive: `c_directive(path, i, kind, start, end, line)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Directive {
    pub kind: Kind,
    /// From the `#` to its line's last token's end.
    pub span: Span,
    /// The line of its `#`, from 1, in the file.
    pub line: u32,
    /// The group it sits in: the conditional directive that opened it (`None`: the file's top).
    pub parent: Option<u32>,
}

/// `c_define(path, i, name, fnlike, params, variadic, body_start, body_end)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Define {
    pub i: u32,
    pub name: Span,
    /// Function-like: a `(` right after the name.
    pub fnlike: bool,
    pub params: Vec<Span>,
    pub variadic: Variadic,
    /// The replacement list (empty where there is none).
    pub body: Span,
}

/// A function-like macro's variadic parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variadic {
    No,
    /// `...`: the arguments are `__VA_ARGS__`.
    Standard,
    /// GNU's `name...`: the arguments are `name` (the last of `params`).
    Named,
}

/// How an include names its header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// `"…"`.
    Quote,
    /// `<…>`.
    Angle,
    /// Tokens, which make a header name once their macros are expanded.
    Macro,
}

/// `c_include(path, i, kind, form, spelled)`: `#include`, `#include_next`, `#import` and `#embed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Include {
    pub i: u32,
    pub kind: Kind,
    pub form: Form,
    /// The header name, its quotes or brackets included; or the tokens (`Form::Macro`). `#embed`'s
    /// parameters after it are not in it.
    pub spelled: Span,
}

/// `c_cond(path, i, kind, expr)`: a conditional directive; `expr` is `#if`'s and `#elif`'s
/// expression, `#ifdef`'s family's name, and empty for `#else` and `#endif`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cond {
    pub i: u32,
    pub kind: Kind,
    pub expr: Span,
    /// The next directive of its chain (`c_cond_next`): an `#elif…`, `#else` or `#endif`.
    pub next: Option<u32>,
}

/// `c_pragma(path, i, first, second, text)`: a `#pragma`'s first word (`once`, `GCC`, `clang`,
/// `STDC`, `omp`, `push_macro`…), its second where there is one, and all of its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pragma {
    pub i: u32,
    pub first: Span,
    pub second: Span,
    pub text: Span,
}

/// `c_line(path, i, n, file)`: `#line` and GNU's line markers. `n` is `None` where the line's
/// tokens are macros (the text is `rest`); the flags of a line marker follow its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineDir {
    pub i: u32,
    pub n: Option<u64>,
    pub file: Span,
    pub rest: Span,
}

/// `#error`, `#warning`, `#ident`, `#sccs`, `#assert`, `#unassert`, and an unknown directive: the
/// text after its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Text {
    pub i: u32,
    pub text: Span,
}

/// What the file gets wrong (`c_pp_diag` in mode (a)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// An `#elif…`, `#else` or `#endif` with no `#if…` open.
    Unmatched(u32),
    /// An `#elif…` or `#else` after its chain's `#else`.
    AfterElse(u32),
    /// An `#if…` its file does not close.
    Unterminated(u32),
    /// A directive whose operand is not one it takes (a `#define` with no name, a parameter list
    /// left open).
    Malformed(u32),
    /// A block comment the file does not close.
    UnclosedComment,
}

/// A file's directives.
pub struct Directives<'s> {
    pub p2: Phase2<'s>,
    pub all: Vec<Directive>,
    pub defines: Vec<Define>,
    /// `#undef`'s name, by directive.
    pub undefs: Vec<(u32, Span)>,
    pub includes: Vec<Include>,
    pub conds: Vec<Cond>,
    pub pragmas: Vec<Pragma>,
    pub lines: Vec<LineDir>,
    pub texts: Vec<Text>,
    pub problems: Vec<Problem>,
    /// The include guard's macro, and its `#ifndef`'s index.
    pub guard: Option<(u32, Span)>,
}

impl<'s> Directives<'s> {
    /// The text of `s` (the phase-2 text: no `\`-newlines).
    pub fn text(&self, s: Span) -> &[u8] {
        &self.p2.text[s.start as usize..s.end as usize]
    }

    /// The file's bytes of `s`.
    pub fn file(&self, s: Span) -> (u32, u32) {
        if s.is_empty() {
            let at = self.p2.orig(s.start as usize) as u32;
            return (at, at);
        }
        (self.p2.orig(s.start as usize) as u32, self.p2.orig(s.end as usize - 1) as u32 + 1)
    }
}

/// The directives of `src`, read as `opts` say.
pub fn scan(src: &[u8], opts: Options) -> Directives<'_> {
    let p2 = Phase2::of(src, opts.trigraphs);
    // The lexer borrows the text while the rows are made; the rows hold spans only.
    let text: &[u8] = unsafe { std::slice::from_raw_parts(p2.text.as_ptr(), p2.text.len()) };
    let mut d = Directives {
        p2,
        all: Vec::new(),
        defines: Vec::new(),
        undefs: Vec::new(),
        includes: Vec::new(),
        conds: Vec::new(),
        pragmas: Vec::new(),
        lines: Vec::new(),
        texts: Vec::new(),
        problems: Vec::new(),
        guard: None,
    };
    Scanner { lx: PpLexer::new(text, opts), text, src, line: crate::tape::LineCount::default(), stack: Vec::new(), guard: Guard::Start }.run(&mut d);
    d
}

/// How far the include guard's pattern has been seen.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Guard {
    /// Nothing yet: the next must be the guard's `#ifndef`.
    Start,
    /// Its `#ifndef` (directive `i`, name), open.
    Open(u32, Span),
    /// Its `#endif`: nothing may follow.
    Closed(u32, Span),
    /// Not a guarded file.
    No,
}

/// An open conditional: its chain's last directive, and whether `#else` was seen.
struct Open {
    head: u32,
    last: u32,
    cond: usize,
    seen_else: bool,
}

struct Scanner<'t> {
    lx: PpLexer<'t>,
    text: &'t [u8],
    src: &'t [u8],
    line: crate::tape::LineCount,
    stack: Vec<Open>,
    guard: Guard,
}

impl<'t> Scanner<'t> {
    fn run(&mut self, d: &mut Directives) {
        loop {
            let t = self.lx.next_token();
            if t.kind == P::Eof {
                if t.flags & rerail_c_lean::pp::UNCLOSED_COMMENT != 0 {
                    d.problems.push(Problem::UnclosedComment);
                }
                break;
            }
            if t.flags & BOL != 0 && t.kind == P::Hash {
                self.directive(d, t);
            } else {
                // A token outside the directives: inside a guard's group it is the guarded text;
                // before its `#ifndef` or after its `#endif` the file is no guarded one.
                if matches!(self.guard, Guard::Start | Guard::Closed(..)) {
                    self.guard = Guard::No;
                }
                self.lx.skip_line();
            }
        }
        for o in self.stack.drain(..) {
            d.problems.push(Problem::Unterminated(o.head));
        }
        if let Guard::Closed(i, name) = self.guard {
            d.guard = Some((i, name));
        }
    }

    /// The directive whose `#` is `hash`.
    fn directive(&mut self, d: &mut Directives, hash: PpTok) {
        let i = d.all.len() as u32;
        let parent = self.stack.last().map(|o| o.last);
        let line = self.line.of(self.src, d.p2.orig(hash.start as usize));
        let name = if self.lx.at_line_end() { None } else { Some(self.lx.next_token()) };
        let kind = match name {
            None => Kind::Null,
            Some(n) if n.kind == P::Ident => match DIRECTIVE_WORDS.find(self.text, n.start as usize, n.end as usize) {
                0 => Kind::Unknown,
                c => KINDS[c as usize - 1],
            },
            Some(n) if n.kind == P::Number => Kind::LineMarker,
            Some(_) => Kind::Unknown,
        };
        let mut dir = Directive { kind, span: Span::of(&hash), line, parent };
        // The conditional directives' parent is the group their chain sits in.
        if matches!(kind, Kind::Elif | Kind::Elifdef | Kind::Elifndef | Kind::Else | Kind::Endif) {
            dir.parent = self.stack.len().checked_sub(2).map(|k| self.stack[k].last);
        }
        d.all.push(dir);
        // Outside the guard's group, nothing but its `#ifndef` (`cond` opens it) keeps a file
        // guarded.
        match self.guard {
            Guard::Start if !matches!(kind, Kind::If | Kind::Ifndef) => self.guard = Guard::No,
            Guard::Closed(..) => self.guard = Guard::No,
            _ => {}
        }
        let end = match kind {
            Kind::Null => hash.end,
            _ => self.operands(d, i, kind, name.unwrap()),
        };
        d.all[i as usize].span.end = end;
    }

    /// The operands of directive `i` after its name `name`, read to its line's end: the line's last
    /// token's end.
    fn operands(&mut self, d: &mut Directives, i: u32, kind: Kind, name: PpTok) -> u32 {
        let mut end = name.end;
        let toks = |lx: &mut PpLexer, end: &mut u32| -> Option<PpTok> {
            if lx.at_line_end() {
                return None;
            }
            let t = lx.next_token();
            *end = t.end;
            Some(t)
        };
        match kind {
            Kind::Define => {
                let Some(m) = toks(&mut self.lx, &mut end).filter(|t| t.kind == P::Ident) else {
                    d.problems.push(Problem::Malformed(i));
                    self.lx.skip_line();
                    return self.line_end(end);
                };
                let mut def = Define { i, name: Span::of(&m), fnlike: false, params: Vec::new(), variadic: Variadic::No, body: Span { start: m.end, end: m.end } };
                let mut first = toks(&mut self.lx, &mut end);
                if first.is_some_and(|t| t.is(rerail_c_lean::T::LParen) && t.flags & SPACE == 0) {
                    def.fnlike = true;
                    if !self.params(&mut def, &mut end) {
                        d.problems.push(Problem::Malformed(i));
                        d.defines.push(def);
                        self.lx.skip_line();
                        return self.line_end(end);
                    }
                    first = toks(&mut self.lx, &mut end);
                }
                if let Some(b) = first {
                    let mut body = Span::of(&b);
                    while let Some(t) = toks(&mut self.lx, &mut end) {
                        body = body.to(&t);
                    }
                    def.body = body;
                }
                d.defines.push(def);
                return end;
            }
            Kind::Undef => match toks(&mut self.lx, &mut end) {
                Some(t) if t.kind == P::Ident => d.undefs.push((i, Span::of(&t))),
                _ => d.problems.push(Problem::Malformed(i)),
            },
            k if k.is_include() => {
                if let Some(h) = self.lx.header_name() {
                    end = h.end;
                    let form = if self.text[h.start as usize] == b'<' { Form::Angle } else { Form::Quote };
                    d.includes.push(Include { i, kind, form, spelled: Span::of(&h) });
                } else {
                    match toks(&mut self.lx, &mut end) {
                        Some(f) => {
                            let mut s = Span::of(&f);
                            // `#embed`'s parameters (`limit(…)`, `prefix(…)`) are not the header's.
                            if kind != Kind::Embed {
                                while let Some(t) = toks(&mut self.lx, &mut end) {
                                    s = s.to(&t);
                                }
                            }
                            d.includes.push(Include { i, kind, form: Form::Macro, spelled: s });
                        }
                        None => d.problems.push(Problem::Malformed(i)),
                    }
                }
            }
            Kind::If | Kind::Elif | Kind::Ifdef | Kind::Ifndef | Kind::Elifdef | Kind::Elifndef | Kind::Else | Kind::Endif => {
                let mut expr = Span { start: name.end, end: name.end };
                let mut ifndef_name = None;
                if let Some(f) = toks(&mut self.lx, &mut end) {
                    expr = Span::of(&f);
                    // A guard's test is five tokens at most (`! defined ( X )`).
                    let mut first = vec![f];
                    while let Some(t) = toks(&mut self.lx, &mut end) {
                        expr = expr.to(&t);
                        if first.len() <= 5 {
                            first.push(t);
                        }
                    }
                    ifndef_name = guard_name(kind, &first, self.text);
                }
                self.cond(d, i, kind, expr, ifndef_name);
            }
            Kind::Pragma => {
                let mut p = Pragma { i, first: Span::default(), second: Span::default(), text: Span { start: name.end, end: name.end } };
                if let Some(f) = toks(&mut self.lx, &mut end) {
                    p.text = Span::of(&f);
                    if f.kind == P::Ident {
                        p.first = Span::of(&f);
                    }
                    let mut k = 0;
                    while let Some(t) = toks(&mut self.lx, &mut end) {
                        if k == 0 && t.kind == P::Ident && f.kind == P::Ident {
                            p.second = Span::of(&t);
                        }
                        k += 1;
                        p.text = p.text.to(&t);
                    }
                }
                d.pragmas.push(p);
            }
            Kind::Line | Kind::LineMarker => {
                let mut l = LineDir { i, n: None, file: Span::default(), rest: Span { start: name.end, end: name.end } };
                let num = if kind == Kind::LineMarker { Some(name) } else { toks(&mut self.lx, &mut end) };
                if let Some(nt) = num {
                    l.rest = Span::of(&nt);
                    if nt.kind == P::Number {
                        l.n = std::str::from_utf8(&self.text[nt.start as usize..nt.end as usize]).ok().and_then(|s| s.parse().ok());
                    }
                    let mut k = 0;
                    while let Some(t) = toks(&mut self.lx, &mut end) {
                        if k == 0 && t.kind == P::Str {
                            l.file = Span::of(&t);
                        }
                        k += 1;
                        l.rest = l.rest.to(&t);
                    }
                }
                d.lines.push(l);
            }
            _ => {
                // `#error`, `#warning`, `#ident`, `#assert` and the rest: the text after the name.
                let mut s = Span { start: name.end, end: name.end };
                if let Some(f) = toks(&mut self.lx, &mut end) {
                    s = Span::of(&f);
                    while let Some(t) = toks(&mut self.lx, &mut end) {
                        s = s.to(&t);
                    }
                }
                d.texts.push(Text { i, text: s });
            }
        }
        // The tokens no operand takes (`#undef X junk`, `#embed`'s parameters): the directive
        // still ends at its line's last token.
        while toks(&mut self.lx, &mut end).is_some() {}
        self.lx.skip_line();
        end
    }

    /// A directive's end where its operands were left unread: the line's last token's.
    fn line_end(&self, end: u32) -> u32 {
        end.max(self.lx.pos as u32)
    }

    /// A function-like macro's parameters after its `(`, to its `)`: `false` where the list is not
    /// one (left open, or a token that is no parameter).
    fn params(&mut self, def: &mut Define, end: &mut u32) -> bool {
        use rerail_c_lean::T;
        let mut want_param = true;
        loop {
            if self.lx.at_line_end() {
                return false;
            }
            let t = self.lx.next_token();
            *end = t.end;
            if t.is(T::RParen) {
                // `()`, or a list that does not end in a `,`.
                return !want_param || def.params.is_empty() && def.variadic == Variadic::No;
            }
            if def.variadic != Variadic::No {
                return false;
            }
            if want_param && t.kind == P::Ident {
                def.params.push(Span::of(&t));
                want_param = false;
                // GNU's named variadic: `name...`.
                if !self.lx.at_line_end() {
                    let save = self.lx.pos;
                    let n = self.lx.next_token();
                    if n.is(T::DotDotDot) {
                        *end = n.end;
                        def.variadic = Variadic::Named;
                    } else {
                        self.lx.seek(save, false);
                    }
                }
            } else if want_param && t.is(T::DotDotDot) {
                def.variadic = Variadic::Standard;
                want_param = false;
            } else if !want_param && t.is(T::Comma) {
                want_param = true;
            } else {
                return false;
            }
        }
    }

    /// Conditional directive `i`: its chain and nesting, and the include guard's pattern.
    fn cond(&mut self, d: &mut Directives, i: u32, kind: Kind, expr: Span, ifndef_name: Option<Span>) {
        let k = d.conds.len();
        d.conds.push(Cond { i, kind, expr, next: None });
        match kind {
            Kind::If | Kind::Ifdef | Kind::Ifndef => {
                if self.stack.is_empty() {
                    self.guard = match (self.guard, ifndef_name) {
                        (Guard::Start, Some(name)) => Guard::Open(i, name),
                        _ => Guard::No,
                    };
                }
                self.stack.push(Open { head: i, last: i, cond: k, seen_else: false });
            }
            _ => {
                let Some(top) = self.stack.last_mut() else {
                    d.problems.push(Problem::Unmatched(i));
                    return;
                };
                if top.seen_else && kind != Kind::Endif {
                    d.problems.push(Problem::AfterElse(i));
                }
                d.conds[top.cond].next = Some(i);
                top.last = i;
                top.cond = k;
                if kind == Kind::Else {
                    top.seen_else = true;
                }
                if kind == Kind::Endif {
                    self.stack.pop();
                    if self.stack.is_empty() {
                        self.guard = match self.guard {
                            Guard::Open(g, name) => Guard::Closed(g, name),
                            _ => Guard::No,
                        };
                    }
                } else if self.stack.len() == 1 {
                    // An `#elif` or `#else` on the guard's own chain: no guard.
                    self.guard = Guard::No;
                }
            }
        }
    }
}

/// The guard's macro of `#ifndef X`, `#if !defined X` or `#if !defined(X)`, from its tokens.
fn guard_name(kind: Kind, t: &[PpTok], text: &[u8]) -> Option<Span> {
    use rerail_c_lean::T;
    let is_defined = |t: &PpTok| t.kind == P::Ident && &text[t.start as usize..t.end as usize] == b"defined";
    match (kind, t) {
        (Kind::Ifndef, [n]) if n.kind == P::Ident => Some(Span::of(n)),
        (Kind::If, [b, d, n]) if b.is(T::Bang) && is_defined(d) && n.kind == P::Ident => Some(Span::of(n)),
        (Kind::If, [b, d, l, n, r]) if b.is(T::Bang) && is_defined(d) && l.is(T::LParen) && n.kind == P::Ident && r.is(T::RParen) => Some(Span::of(n)),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
