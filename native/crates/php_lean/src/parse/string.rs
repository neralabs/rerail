//! Strings: single-quoted (`string`), double-quoted (`encapsed_string`), shell commands, heredocs
//! and nowdocs, taken apart from the one token the lexer reads for each.
//!
//! **A body's pieces** are cut where tree-sitter-php's external scanner (`scanner.h`,
//! `scan_encapsed_part_string`) ends a `string_content`: before a quote that closes, before `\`
//! and a character it escapes, before `$` and a name's first character or `{`, before `{$`, and
//! (in a heredoc) before a line's end. Right after a variable, `[` and `->` and a name continue it
//! (a `subscript_expression`, a `member_access_expression`). An interpolation's expression
//! (`{$…}`, `${…}`) is lexed again as tokens (`relex_at`) and parsed as any other.
//!
//! **Quotes.** The opening quote's gap is no implied word (a `b` prefix makes it a hidden token),
//! so it is stored; the closing one is dropped, but an empty string keeps both.

use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Double,
    Shell,
    Heredoc,
}

fn is_name_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

/// An escape sequence's length from its `\` at `s` (tree-sitter-php's `escape_sequence`), or
/// `None` where there is none (`\u` with no braces: a `string_content` of its own).
fn escape_len(src: &[u8], s: usize) -> Option<usize> {
    let at = |i: usize| src.get(i).copied().unwrap_or(0);
    match at(s + 1) {
        b'n' | b'r' | b't' | b'v' | b'e' | b'f' | b'\\' | b'$' | b'"' | b'`' => Some(2),
        b'0'..=b'7' => {
            let mut k = 2;
            while k < 4 && matches!(at(s + k), b'0'..=b'7') {
                k += 1;
            }
            Some(k)
        }
        b'x' if at(s + 2).is_ascii_hexdigit() => Some(if at(s + 3).is_ascii_hexdigit() { 4 } else { 3 }),
        b'u' if at(s + 2) == b'{' => {
            let mut k = 3;
            while at(s + k).is_ascii_hexdigit() {
                k += 1;
            }
            (k > 3 && at(s + k) == b'}').then_some(k + 1)
        }
        _ => None,
    }
}

impl<'a> P<'a> {
    /// A node over `start..end` with no children, in field `field` and class `class`.
    pub(super) fn mark(&mut self, kind: K, field: u8, class: u8, start: usize, end: usize) {
        let at = self.w.push(key(kind, field, class, false), start as u32);
        self.w.close(at, end as u32);
    }

    /// Token `tok` over `start..end` under `parent`, where the tape stores it (or `keep`).
    fn tok_piece(&mut self, parent: K, tok: K, field: u8, start: usize, end: usize, keep: bool) {
        if keep || self.stores(parent, tok, field) {
            self.mark(tok, field, C_NONE, start, end);
        }
    }

    /// The string token at the parser's position, taken: its extras before it written, and lexing
    /// resumed at its end once its pieces are read (`done`).
    fn take_string(&mut self) -> R<Tok> {
        let t = self.tok();
        self.flush(t.start)?;
        if matches!(self.src[t.start as usize], b'b' | b'B') {
            // `b'…'`, `b"…"`: the prefix's token is hidden (not modelled).
            return self.refuse();
        }
        Ok(t)
    }

    /// Lexing resumed after a string that ended at `end`.
    fn done(&mut self, end: u32) {
        self.relex_at(end as usize);
        self.last_end = end;
    }

    /// A token lexed at `pos` with nothing before it (white space or a comment), or a refusal.
    fn token_here(&mut self, pos: usize) -> R<Tok> {
        let mut extras = Vec::new();
        match self.lx.token(pos, &mut extras) {
            Ok(t) if t.start as usize == pos && extras.is_empty() => Ok(t),
            _ => {
                self.far = self.far.max(pos as u32);
                Err(Refusal::Parse(pos as u32))
            }
        }
    }

    /// A refusal at byte `pos`.
    fn refuse_at<X>(&mut self, pos: usize) -> R<X> {
        self.far = self.far.max(pos as u32);
        Err(Refusal::Parse(pos as u32))
    }

    /// `string`: `'`, then `\\` and `\'` (`escape_sequence`) and runs of anything else
    /// (`string_content`, one node over its tokens), then `'`.
    pub(super) fn single_string(&mut self, s: Slot) -> R<N> {
        let t = self.take_string()?;
        let (start, end) = (t.start as usize, t.end as usize);
        let n = self.open_at(K::String, s.field, class(K::String, s.ctx), t.start);
        let empty = end - start == 2;
        self.tok_piece(K::String, K::TQuote, f::NONE, start, start + 1, true);
        let close = end - 1;
        let mut i = start + 1;
        while i < close {
            if self.src[i] == b'\\' && matches!(self.src[i + 1], b'\\' | b'\'') {
                self.mark(K::EscapeSequence, f::NONE, C_NONE, i, i + 2);
                i += 2;
                continue;
            }
            // `\\?[^'\\]+`, repeated.
            let from = i;
            loop {
                let mut k = i;
                if self.src[k] == b'\\' {
                    k += 1;
                }
                let e = self.lx.blocks.quote_stop(k).min(close);
                if e == k {
                    break;
                }
                i = e;
                if i >= close || matches!(self.src[i + 1], b'\\' | b'\'') {
                    break;
                }
            }
            if i == from {
                // `\` before the closing quote: no token takes it.
                return self.refuse_at(i);
            }
            self.mark(K::StringContent, f::NONE, C_NONE, from, i);
        }
        self.tok_piece(K::String, K::TQuote, f::NONE, close, end, empty);
        self.w.close(n.at, t.end);
        self.done(t.end);
        Ok(n)
    }

    /// `encapsed_string`: `"`, its interpolated body, `"`.
    pub(super) fn double_string(&mut self, s: Slot) -> R<N> {
        let t = self.take_string()?;
        let (start, end) = (t.start as usize, t.end as usize);
        let n = self.open_at(K::EncapsedString, s.field, class(K::EncapsedString, s.ctx), t.start);
        let empty = end - start == 2;
        self.tok_piece(K::EncapsedString, K::TDQuote, f::NONE, start, start + 1, true);
        self.interpolated(K::EncapsedString, start + 1, end - 1, Mode::Double, &[])?;
        self.tok_piece(K::EncapsedString, K::TDQuote, f::NONE, end - 1, end, empty);
        self.w.close(n.at, t.end);
        self.done(t.end);
        Ok(n)
    }

    /// `shell_command_expression`: `` ` ``, its interpolated body, `` ` ``.
    pub(super) fn shell_command(&mut self, s: Slot) -> R<N> {
        let t = self.take_string()?;
        let (start, end) = (t.start as usize, t.end as usize);
        let n = self.open_at(K::ShellCommandExpression, s.field, class(K::ShellCommandExpression, s.ctx), t.start);
        let empty = end - start == 2;
        self.tok_piece(K::ShellCommandExpression, K::TBacktick, f::NONE, start, start + 1, empty);
        self.interpolated(K::ShellCommandExpression, start + 1, end - 1, Mode::Shell, &[])?;
        self.tok_piece(K::ShellCommandExpression, K::TBacktick, f::NONE, end - 1, end, empty);
        self.w.close(n.at, t.end);
        self.done(t.end);
        Ok(n)
    }

    /// `heredoc` or `nowdoc`: `<<<`, its word (quoted for a nowdoc, maybe for a heredoc), its body's
    /// lines, its end tag.
    pub(super) fn heredoc(&mut self, s: Slot) -> R<N> {
        let t = self.take_string()?;
        let start = t.start as usize;
        let Some((nowdoc, ws, we, body)) = self.lx.heredoc_open(start) else { return self.refuse() };
        let word = &self.src[ws..we];
        let kind = if nowdoc { K::Nowdoc } else { K::Heredoc };
        let n = self.open_at(kind, s.field, class(kind, s.ctx), t.start);
        self.tok_piece(kind, K::TLtLtLt, f::NONE, start, start + 3, false);
        let quoted = self.src[ws - 1] == b'"' || nowdoc;
        let quote = if nowdoc { K::TQuote } else { K::TDQuote };
        if quoted {
            self.tok_piece(kind, quote, f::NONE, ws - 1, ws, false);
        }
        self.mark(K::HeredocStart, f::IDENTIFIER, C_NONE, ws, we);
        if quoted {
            self.tok_piece(kind, quote, f::NONE, we, we + 1, false);
        }
        // The opening line's end: the body's first token.
        let nl = if self.src[body - 1] == b'\n' && body >= 2 && self.src[body - 2] == b'\r' { body - 2 } else { body - 1 };
        // The end tag's line.
        let mut line = body;
        let (tag_start, tag_end) = loop {
            if let Some(tag) = self.lx.end_tag(line, word) {
                break tag;
            }
            let e = self.lx.blocks.eol(line);
            if e >= self.src.len() {
                return self.refuse();
            }
            line = if self.src[e] == b'\r' && self.src.get(e + 1) == Some(&b'\n') { e + 2 } else { e + 1 };
        };
        // The body ends before the end tag line's line break.
        let limit = if line >= body {
            let mut l = line - 1;
            if self.src[l] == b'\n' && l > 0 && self.src[l - 1] == b'\r' {
                l -= 1;
            }
            l.max(body)
        } else {
            body
        };
        let word = word.to_vec();
        if nowdoc {
            self.nowdoc_body(nl, body, line, &word)?;
        } else if line > body {
            // The pieces first (into a body opened at the line break), the body dropped where it
            // has none.
            let b = self.open_at(K::HeredocBody, f::VALUE, C_NONE, nl as u32);
            let before = self.w.len();
            let last = self.heredoc_lines(body, limit, &word)?;
            if self.w.len() == before {
                self.w.truncate(before - 1);
            } else {
                self.w.close(b.at, last as u32);
            }
        }
        self.mark(K::HeredocEnd, f::END_TAG, C_NONE, tag_start, tag_end);
        self.w.close(n.at, tag_end as u32);
        if tag_end as u32 != t.end {
            return self.refuse();
        }
        self.done(t.end);
        Ok(n)
    }

    /// A heredoc's lines from `from` to `limit` (before the end tag line's break): their pieces,
    /// blank lines passed. Where its last piece ends.
    fn heredoc_lines(&mut self, from: usize, limit: usize, word: &[u8]) -> R<usize> {
        let mut pos = from;
        let mut last = from;
        while pos < limit {
            match self.src[pos] {
                b'\n' => {
                    pos += 1;
                    continue;
                }
                b'\r' => {
                    pos += if self.src.get(pos + 1) == Some(&b'\n') { 2 } else { 1 };
                    continue;
                }
                _ => {}
            }
            let e = self.interpolated_line(K::HeredocBody, pos, limit, word)?;
            last = self.last_end.max(e as u32) as usize;
            pos = e;
        }
        Ok(last)
    }

    /// One run of a heredoc's pieces from `pos`, to its line's end or past it (an interpolation's
    /// expression may span lines). Where it stopped.
    fn interpolated_line(&mut self, parent: K, pos: usize, limit: usize, word: &[u8]) -> R<usize> {
        let e = self.lx.blocks.eol(pos).min(limit);
        let stop = self.interpolated(parent, pos, e, Mode::Heredoc, word)?;
        Ok(stop)
    }

    /// `nowdoc_body`: the opening line's break, then `nowdoc_string`s, each from the end of the one
    /// before (white space and line breaks included) to its line's end.
    fn nowdoc_body(&mut self, nl: usize, body: usize, tag_line: usize, _word: &[u8]) -> R<()> {
        let mut pieces = Vec::new();
        let mut pos = body;
        loop {
            let mut q = pos;
            while q < tag_line && matches!(self.src[q], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
                q += 1;
            }
            if q >= tag_line {
                break;
            }
            let e = self.lx.blocks.eol(q);
            pieces.push((pos, e));
            pos = e;
        }
        if pieces.is_empty() {
            return Ok(());
        }
        let last = pieces.last().unwrap().1;
        let b = self.open_at(K::NowdocBody, f::VALUE, C_NONE, nl as u32);
        for (s, e) in pieces {
            self.mark(K::NowdocString, f::NONE, C_NONE, s, e);
        }
        self.w.close(b.at, last as u32);
        Ok(())
    }

    /// An interpolated body's pieces from `from` to `to`, under `parent` (`_interpolated_string_body`
    /// and its heredoc and shell forms). Where the pieces end (past `to` where an interpolation's
    /// expression runs on, in a heredoc).
    fn interpolated(&mut self, parent: K, from: usize, to: usize, mode: Mode, word: &[u8]) -> R<usize> {
        let src = self.src;
        let mut i = from;
        let mut to = to;
        let mut after_variable = false;
        // Where the last interpolation in braces ended.
        let mut braced_end: Option<usize> = None;
        while i < to {
            let content = i;
            // A heredoc's call starts past white space, where an end tag would end the heredoc (one
            // the lexer did not end it at: refused).
            if mode == Mode::Heredoc {
                let mut k = i;
                while k < to && matches!(src[k], b' ' | b'\t' | 0x0b | 0x0c) {
                    k += 1;
                }
                if self.tag_ends_at(k, word) {
                    return self.refuse_at(k);
                }
                if after_variable && k > i && (src.get(k) == Some(&b'[') || src[k..].starts_with(b"->")) {
                    return self.refuse_at(k);
                }
                i = k;
            }
            // The content up to the next stop.
            loop {
                if i >= to {
                    break;
                }
                let next = self.lx.blocks.string_stop(i).min(to);
                i = next;
                if i >= to {
                    break;
                }
                let c = src[i];
                let d = src.get(i + 1).copied().unwrap_or(0);
                let stop = match c {
                    b'"' => mode == Mode::Double,
                    b'`' => mode == Mode::Shell,
                    b'\\' => match d {
                        b'{' => {
                            i += 2;
                            continue;
                        }
                        b'`' if mode == Mode::Shell => true,
                        b'\\' if mode == Mode::Heredoc => {
                            i += 2;
                            continue;
                        }
                        b'n' | b'r' | b't' | b'v' | b'e' | b'f' | b'\\' | b'$' | b'"' | b'u' | b'0'..=b'7' => true,
                        b'x' => src.get(i + 2).is_some_and(|h| h.is_ascii_hexdigit()),
                        _ => false,
                    },
                    b'$' => (is_name_byte(d) && !d.is_ascii_digit()) || d == b'{',
                    b'{' => d == b'$',
                    _ => false,
                };
                if stop {
                    break;
                }
                i += 1;
            }
            if i > content {
                braced_end = None;
                self.mark(K::StringContent, f::NONE, C_NONE, content, i);
                self.last_end = i as u32;
                after_variable = false;
                continue;
            }
            if i >= to {
                break;
            }
            // A piece at a stop.
            after_variable = false;
            if src[i] != b'{' {
                braced_end = None;
            }
            match src[i] {
                b'"' | b'`' => return self.refuse_at(i),
                b'\\' => match escape_len(src, i) {
                    Some(k) => {
                        self.mark(K::EscapeSequence, f::NONE, C_NONE, i, i + k);
                        i += k;
                    }
                    None if src[i + 1] == b'u' => {
                        self.mark(K::StringContent, f::NONE, C_NONE, i, i + 2);
                        i += 2;
                    }
                    None => return self.refuse_at(i),
                },
                b'$' if src[i + 1] == b'{' => {
                    // `${expression}`: a `dynamic_variable_name`.
                    self.relex_at(i);
                    self.simple_variable(BARE)?;
                    i = self.last_end as usize;
                }
                b'$' => {
                    let at = self.variable_in_string(parent, i)?;
                    i = at.0;
                    after_variable = at.1;
                }
                b'{' => {
                    // `{expression}`.
                    // Its `{` is stored before an expression that could stand without braces (a
                    // simple part's kinds): there the gap's shortest word is empty.
                    // At an encapsed string's start it is stored too (a `b"` prefix is hidden: the
                    // gap's shortest word is the `{` alone). Its `}` is dropped but before `{` and a
                    // simple part's kind (`}{`, where `}` alone is the shortest).
                    self.relex_at(i);
                    let before = (braced_end == Some(i)).then(|| i as u32 - 1);
                    let start = parent == K::EncapsedString && i == from;
                    self.braced(EXPR, before, |k| start || matches!(k, K::VariableName | K::DynamicVariableName | K::MemberAccessExpression | K::SubscriptExpression))?;
                    if self.peek() != T::RBrace {
                        return self.refuse();
                    }
                    self.bump()?;
                    i = self.last_end as usize;
                    braced_end = Some(i);
                    self.last_end = i as u32;
                    self.relex_at(i);
                    if i > to {
                        if mode != Mode::Heredoc {
                            return self.refuse_at(i);
                        }
                        to = self.lx.blocks.eol(i).max(to);
                    }
                    continue;
                }
                _ => return self.refuse_at(i),
            }
            self.last_end = i as u32;
            // Tokens and extras lexed ahead of an interpolation are the string's bytes: dropped.
            self.relex_at(i);
            if i > to {
                // An expression past the line's end (a heredoc's): its line runs on.
                if mode != Mode::Heredoc {
                    return self.refuse_at(i);
                }
                to = self.lx.blocks.eol(i).max(to);
            }
        }
        Ok(i)
    }

    /// Whether heredoc end tag `word` at `k` would end it (scanner.h: the word, then white space,
    /// `;`, `,` or `)`, then white space to one of `;`, `,`, `)` or a line break).
    fn tag_ends_at(&self, k: usize, word: &[u8]) -> bool {
        let src = self.src;
        if word.is_empty() || !src[k..].starts_with(word) {
            return false;
        }
        let mut e = k + word.len();
        let ws = |c: u8| matches!(c, b' ' | b'\t' | 0x0b | 0x0c | b'\n' | b'\r');
        if !src.get(e).is_some_and(|&c| ws(c) || matches!(c, b';' | b',' | b')')) {
            return false;
        }
        while src.get(e).is_some_and(|&c| matches!(c, b' ' | b'\t' | 0x0b | 0x0c)) {
            e += 1;
        }
        src.get(e).is_some_and(|&c| matches!(c, b';' | b',' | b')' | b'\n' | b'\r'))
    }

    /// A variable in a string from its `$` at `s`: `variable_name`, and `[…]` or `->name` right
    /// after it (`_simple_string_part`). Where it ends, and whether it is a bare variable (its
    /// next piece read as after a variable).
    fn variable_in_string(&mut self, parent: K, s: usize) -> R<(usize, bool)> {
        let _ = parent;
        let src = self.src;
        let name = self.token_here(s + 1)?;
        if name.kind != T::Name {
            return self.refuse_at(s + 1);
        }
        let e = name.end as usize;
        let at = src.get(e).copied().unwrap_or(0);
        // A `subscript_expression`: `[` an integer, `-` an integer, a name or a variable `]`.
        if at == b'[' {
            let n = self.open_at(K::SubscriptExpression, f::NONE, C_NONE, s as u32);
            self.var_node(s, &name);
            self.tok_piece(K::SubscriptExpression, K::TLBracket, f::NONE, e, e + 1, false);
            let mut k = e + 1;
            let t = self.token_here(k)?;
            match t.kind {
                T::Int => {
                    self.mark(K::Integer, f::NONE, C_NONE, k, t.end as usize);
                    k = t.end as usize;
                }
                T::Minus => {
                    let i = self.token_here(t.end as usize)?;
                    if i.kind != T::Int {
                        return self.refuse_at(k);
                    }
                    let u = self.open_at(K::UnaryOpExpression, f::NONE, C_NONE, k as u32);
                    self.tok_piece(K::UnaryOpExpression, K::TMinus, f::NONE, k, k + 1, false);
                    self.mark(K::Integer, f::NONE, C_NONE, i.start as usize, i.end as usize);
                    self.w.close(u.at, i.end);
                    k = i.end as usize;
                }
                T::Name if !w::is_reserved(t.word) => {
                    self.mark(K::Name, f::NONE, C_NONE, k, t.end as usize);
                    k = t.end as usize;
                }
                T::Dollar => {
                    let v = self.token_here(k + 1)?;
                    if v.kind != T::Name {
                        return self.refuse_at(k);
                    }
                    self.var_node(k, &v);
                    k = v.end as usize;
                }
                _ => return self.refuse_at(k),
            }
            if src.get(k) != Some(&b']') {
                return self.refuse_at(k);
            }
            self.tok_piece(K::SubscriptExpression, K::TRBracket, f::NONE, k, k + 1, false);
            self.w.close(n.at, (k + 1) as u32);
            return Ok((k + 1, false));
        }
        // A `member_access_expression`: `->` and a name.
        if at == b'-' && src.get(e + 1) == Some(&b'>') && src.get(e + 2).is_some_and(|&c| is_name_byte(c)) {
            let m = self.token_here(e + 2)?;
            if m.kind != T::Name || w::is_reserved(m.word) {
                return self.refuse_at(e + 2);
            }
            let n = self.open_at(K::MemberAccessExpression, f::NONE, C_NONE, s as u32);
            let v = self.w.len();
            self.var_node(s, &name);
            self.w.set_key(v, key(K::VariableName, f::OBJECT, C_NONE, false));
            self.tok_piece(K::MemberAccessExpression, K::TArrow, f::NONE, e, e + 2, false);
            self.mark(K::Name, f::NAME, C_NONE, e + 2, m.end as usize);
            self.w.close(n.at, m.end);
            return Ok((m.end as usize, false));
        }
        self.var_node(s, &name);
        Ok((e, true))
    }

    /// `variable_name` from its `$` at `s`, its name `name`.
    fn var_node(&mut self, s: usize, name: &Tok) {
        let n = self.open_at(K::VariableName, f::NONE, C_NONE, s as u32);
        self.mark(K::Name, f::NONE, C_NONE, name.start as usize, name.end as usize);
        self.w.close(n.at, name.end);
    }
}
