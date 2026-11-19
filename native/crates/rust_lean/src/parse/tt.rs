//! Token trees: a macro invocation's (`delim_token_tree`), a macro rule's pattern
//! (`token_tree_pattern`, with `$x:frag` bindings and `$(…)sep*` repetitions) and its body
//! (`token_tree`, with metavariables and repetitions).
//!
//! In a tree every word is an identifier but those the grammar lists (`_non_special_token`): the
//! keywords it names are tokens, stored; `self`, `super`, `crate`, `mut`, the primitive types and
//! the literals are named. A lifetime is two: `'` and its word. The opener is always stored (three
//! brackets tie); the closer is implied only right after a named child.

use super::*;

/// Which tree.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Tt {
    /// A macro rule's body.
    Tree,
    /// A macro rule's pattern.
    Pattern,
    /// A macro invocation's or an attribute's arguments (`$` is a token there).
    Delim,
}

/// A word in a token tree: its kind, and whether it is a named child.
fn tt_word(word: u8, underscore: bool) -> (K, bool) {
    if underscore {
        return (K::TUnderscore, false);
    }
    let k = match word {
        w::SELF => return (K::SelfK, true),
        w::SUPER => return (K::Super, true),
        w::CRATE => return (K::Crate, true),
        w::MUT => return (K::MutableSpecifier, true),
        c if w::is_primitive(c) => return (K::PrimitiveType, true),
        w::AS => K::TAs,
        w::ASYNC => K::TAsync,
        w::AWAIT => K::TAwait,
        w::BREAK => K::TBreak,
        w::CONST => K::TConst,
        w::CONTINUE => K::TContinue,
        w::DEFAULT => K::TDefault,
        w::ENUM => K::TEnum,
        w::FN => K::TFn,
        w::FOR => K::TFor,
        w::GEN => K::TGen,
        w::IF => K::TIf,
        w::IMPL => K::TImpl,
        w::LET => K::TLet,
        w::LOOP => K::TLoop,
        w::MATCH => K::TMatch,
        w::MOD => K::TMod,
        w::PUB => K::TPub,
        w::RETURN => K::TReturn,
        w::STATIC => K::TStatic,
        w::STRUCT => K::TStruct,
        w::TRAIT => K::TTrait,
        w::TYPE => K::TType,
        w::UNION => K::TUnion,
        w::UNSAFE => K::TUnsafe,
        w::USE => K::TUse,
        w::WHERE => K::TWhere,
        w::WHILE => K::TWhile,
        _ => return (K::Identifier, true),
    };
    (k, false)
}

/// A fragment specifier's token.
fn fragment(word: u8) -> Option<K> {
    Some(match word {
        w::BLOCK => K::TBlock,
        w::EXPR => K::TExpr,
        w::EXPR_2021 => K::TExpr2021,
        w::IDENT => K::TIdent,
        w::ITEM => K::TItem,
        w::LIFETIME => K::TLifetime,
        w::LITERAL => K::TLiteral,
        w::META => K::TMeta,
        w::PAT => K::TPat,
        w::PAT_PARAM => K::TPatParam,
        w::PATH => K::TPath,
        w::STMT => K::TStmt,
        w::TT => K::TTt,
        w::TY => K::TTy,
        w::VIS => K::TVis,
        _ => return None,
    })
}

impl<'a> P<'a> {
    /// A token tree of kind `mode`, in `field`.
    pub(super) fn token_tree(&mut self, mode: Tt, field: u8) -> R<()> {
        self.deeper()?;
        let kind = if mode == Tt::Pattern { K::TokenTreePattern } else { K::TokenTree };
        let close = match self.peek() {
            T::LParen => T::RParen,
            T::LBracket => T::RBracket,
            T::LBrace => T::RBrace,
            _ => return self.refuse(),
        };
        let n = self.open(kind, field, C_NONE);
        self.tk_punct(kind, true)?;
        let mut named = false;
        loop {
            let t = self.tok();
            if t.kind == close {
                self.tk_punct(kind, !named)?;
                break;
            }
            named = self.tt_element(mode)?;
        }
        self.close(n);
        self.depth -= 1;
        Ok(())
    }

    /// One element of a tree: `true` where it is a named child.
    fn tt_element(&mut self, mode: Tt) -> R<bool> {
        let t = self.tok();
        let sub = if mode == Tt::Pattern { Tt::Pattern } else { mode };
        match t.kind {
            T::LParen | T::LBracket | T::LBrace => {
                self.token_tree(sub, f::NONE)?;
                Ok(true)
            }
            T::RParen | T::RBracket | T::RBrace | T::Eof => self.refuse(),
            T::Int => self.leaf(K::IntegerLiteral, f::NONE, C_L).map(|_| true),
            T::Float => self.leaf(K::FloatLiteral, f::NONE, C_L).map(|_| true),
            T::Char => self.leaf(K::CharLiteral, f::NONE, C_L).map(|_| true),
            T::Str | T::RawStr => self.literal(slot(f::NONE, Ctx::Literal)).map(|_| true),
            T::Lifetime => {
                // `'a`: a `'` token and the word.
                let t = self.bump()?;
                self.leaf_span(K::TQuote, f::NONE, C_NONE, t.start, t.start + 1);
                let word = crate::lex::word(self.src, t.start as usize + 1, t.end as usize);
                let underscore = t.end == t.start + 2 && self.src[t.start as usize + 1] == b'_';
                let (k, named) = tt_word(word, underscore);
                self.leaf_span(k, f::NONE, C_NONE, t.start + 1, t.end);
                Ok(named)
            }
            T::Metavariable => match mode {
                Tt::Delim => {
                    // `$x` where no metavariable is read: `$` and the word.
                    let t = self.bump()?;
                    self.leaf_span(K::TDollar, f::NONE, C_NONE, t.start, t.start + 1);
                    let word = crate::lex::word(self.src, t.start as usize + 1, t.end as usize);
                    let underscore = t.end == t.start + 2 && self.src[t.start as usize + 1] == b'_';
                    let (k, named) = tt_word(word, underscore);
                    self.leaf_span(k, f::NONE, C_NONE, t.start + 1, t.end);
                    Ok(named)
                }
                Tt::Pattern if self.peek_at(1).kind == T::Colon => {
                    let b = self.open(K::TokenBindingPattern, f::NONE, C_NONE);
                    self.leaf(K::Metavariable, f::NAME, C_NONE)?;
                    self.tk(K::TokenBindingPattern, K::TColon, f::NONE)?;
                    let t = self.tok();
                    let Some(k) = (t.kind == T::Name).then(|| fragment(t.word)).flatten() else { return self.refuse() };
                    let fs = self.open(K::FragmentSpecifier, f::TYPE, C_NONE);
                    self.leaf(k, f::NONE, C_NONE)?;
                    self.close(fs);
                    self.close(b);
                    Ok(true)
                }
                _ => self.leaf(K::Metavariable, f::NONE, C_NONE).map(|_| true),
            },
            T::Dollar if mode != Tt::Delim => {
                if self.peek_at(1).kind != T::LParen {
                    return self.refuse();
                }
                self.repetition(mode)?;
                Ok(true)
            }
            T::Name => {
                if matches!(t.word, w::TRUE | w::FALSE) {
                    self.literal(slot(f::NONE, Ctx::Literal))?;
                    return Ok(true);
                }
                let (k, named) = tt_word(t.word, self.underscore(t));
                self.leaf(k, f::NONE, C_NONE)?;
                Ok(named)
            }
            _ => {
                let Some(k) = punct(t.kind) else { return self.refuse() };
                self.leaf(k, f::NONE, C_NONE)?;
                Ok(false)
            }
        }
    }

    /// Whether the element at token `k` of a tree is a named child.
    fn tt_named_at(&mut self, k: usize) -> bool {
        let t = self.peek_at(k);
        match t.kind {
            T::LParen | T::LBracket | T::LBrace | T::Int | T::Float | T::Char | T::Str | T::RawStr | T::Metavariable | T::Dollar => true,
            T::Name => matches!(t.word, w::TRUE | w::FALSE) || tt_word(t.word, self.underscore(t)).1,
            _ => false,
        }
    }

    /// `$( … ) sep? op`: the separator (any text but `+`, `*`, `?`) is no token of the tape's.
    fn repetition(&mut self, mode: Tt) -> R<()> {
        let kind = if mode == Tt::Pattern { K::TokenRepetitionPattern } else { K::TokenRepetition };
        let n = self.open(kind, f::NONE, C_NONE);
        self.tk(kind, K::TDollar, f::NONE)?;
        // `(` is implied where a named child follows it.
        let named = self.tt_named_at(1) && self.peek_at(1).kind != T::RParen;
        self.gap_tk(kind, K::TLParen, !named)?;
        while self.peek() != T::RParen {
            self.tt_element(mode)?;
        }
        self.leaf(K::TRParen, f::NONE, C_NONE)?;
        // The separator: from after the `)`, blanks passed, to the first `+`, `*` or `?`.
        let mut i = self.last_end as usize;
        while matches!(self.src.get(i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            i += 1;
        }
        if !matches!(self.src.get(i), Some(b'+' | b'*' | b'?')) {
            let mut q = i;
            while q < self.src.len() && !matches!(self.src[q], b'+' | b'*' | b'?') {
                if self.src[q] == b'/' {
                    return self.refuse_at(q);
                }
                q += 1;
            }
            if q >= self.src.len() {
                return self.refuse_at(i);
            }
            self.relex_at(q);
        }
        let t = self.tok();
        let k = match t.kind {
            T::Plus => K::TPlus,
            T::Star => K::TStar,
            T::Question => K::TQuestion,
            _ => return self.refuse(),
        };
        self.leaf(k, f::NONE, C_NONE)?;
        self.close(n);
        Ok(())
    }
}
