//! P0: a file's tables, once a file whatever its instances: its text tokens, its directives
//! with their lines, and its conditionals' nesting, which is the file's alone (the serial pass
//! keeps a file's conditionals per file, and skips a group by counting its directives' names).
//!
//! The file is read by the lexer the serial pass reads it with, directive by directive as
//! `Pp::directive` reads each kind's line, so its tokens and lines are the serial pass's.

use super::super::{Pp, Src};
use crate::directives::Kind;
use crate::macros::*;
use rerail_c_lean::pp::{P, PpLexer};

/// A file's tables.
pub(super) struct Tab {
    /// The tokens outside directives' lines, in order.
    pub text: Vec<Tk>,
    pub dirs: Vec<Dir>,
    /// The groups: 0 is the file's own; every other is an arm of a chain.
    pub groups: Vec<Group>,
    /// Each chain's arms: their directives (`#if…`, `#elif…`, `#else`), in order.
    pub chains: Vec<Vec<u32>>,
    /// The include guard: where the file is one `#ifndef X` … `#endif` and nothing outside it,
    /// `X`. An include where `X` is defined enters nothing (clang's multiple-include reading).
    pub guard: Option<u32>,
}

/// A group: the group its chain opened in.
pub(super) struct Group {
    pub parent: u32,
}

/// A directive.
pub(super) struct Dir {
    pub kind: Kind,
    pub hash: Tk,
    /// The text tokens before it in the file.
    pub text_before: u32,
    /// Its line's tokens after its name; an include's header name alone, where it has one.
    pub line: Vec<Tk>,
    /// The group it is read in (a chain's directives: the chain's own parent's), and the group
    /// of the text after it.
    pub group: u32,
    pub after: u32,
    pub act: Act,
}

/// What an active directive does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Act {
    Nothing,
    /// Definition `k` (`Pp::defs`), parsed here.
    Define(u32),
    Undef(u32),
    /// An arm's test.
    Test,
    Include,
    /// `#pragma once`, `push_macro`, `pop_macro` (with its name's symbol).
    Once,
    Push(u32),
    Pop(u32),
    Unsupported(&'static str),
}

/// File `file`'s tables.
pub(super) fn tabulate(pp: &mut Pp, file: u32) -> Tab {
    let Src::File { mut lx, .. } = pp.file_src(file) else { unreachable!("a file's source") };
    let mut tab = Tab { text: Vec::new(), dirs: Vec::new(), groups: vec![Group { parent: 0 }], chains: Vec::new(), guard: None };
    // The open chains, and the group read now.
    let mut open: Vec<u32> = Vec::new();
    let mut cur = 0u32;
    loop {
        let pt = lx.next();
        if pt.kind == P::Eof {
            break;
        }
        let t = pp.file_tok(pt, file);
        if t.flags & BOL == 0 || t.kind != P::Hash {
            tab.text.push(t);
            continue;
        }
        let name = lx.next_on_line().map(|n| pp.file_tok(n, file));
        let kind = match name {
            None => Kind::Null,
            Some(n) => match n.kind {
                P::Ident => Kind::of(pp.syms.text(n.sym)),
                P::Number => Kind::LineMarker,
                _ => Kind::Unknown,
            },
        };
        let mut line = Vec::new();
        let mut act = Act::Nothing;
        match kind {
            Kind::Null => {}
            Kind::Include | Kind::IncludeNext | Kind::Import | Kind::Embed => {
                let h = lx.header_name().map(|h| pp.file_tok(h, file));
                let rest = rest_of(&mut lx, pp, file);
                line = match h {
                    Some(h) => vec![h],
                    None => rest,
                };
                act = if kind == Kind::Embed { Act::Unsupported("#embed") } else { Act::Include };
                lx.skip_line();
            }
            Kind::Error | Kind::Warning => {
                lx.rest_of_line();
            }
            Kind::Pragma => {
                let (s, e) = lx.rest_of_line();
                let text = pp.files[file as usize].text()[s..e].to_vec();
                act = pragma(pp, &text);
            }
            _ => {
                line = rest_of(&mut lx, pp, file);
                lx.skip_line();
            }
        }
        let d = tab.dirs.len() as u32;
        let (group, after) = match kind {
            Kind::If | Kind::Ifdef | Kind::Ifndef => {
                let c = tab.chains.len() as u32;
                tab.chains.push(vec![d]);
                open.push(c);
                act = Act::Test;
                let g = tab.groups.len() as u32;
                tab.groups.push(Group { parent: cur });
                let at = cur;
                cur = g;
                (at, g)
            }
            Kind::Elif | Kind::Elifdef | Kind::Elifndef | Kind::Else if !open.is_empty() => {
                let c = *open.last().unwrap();
                let parent = tab.groups[cur as usize].parent;
                tab.chains[c as usize].push(d);
                act = Act::Test;
                let g = tab.groups.len() as u32;
                tab.groups.push(Group { parent });
                cur = g;
                (parent, g)
            }
            Kind::Endif if !open.is_empty() => {
                open.pop();
                cur = tab.groups[cur as usize].parent;
                (cur, cur)
            }
            Kind::Define => {
                if let Some(def) = pp.parse_define(t, file, &line) {
                    pp.defs.push(def);
                    act = Act::Define(pp.defs.len() as u32 - 1);
                }
                (cur, cur)
            }
            Kind::Undef => {
                if let Some(n) = line.first().filter(|n| n.kind == P::Ident) {
                    act = Act::Undef(n.sym);
                }
                (cur, cur)
            }
            Kind::Line | Kind::LineMarker => {
                act = Act::Unsupported("#line");
                (cur, cur)
            }
            _ => (cur, cur),
        };
        tab.dirs.push(Dir { kind, hash: t, text_before: tab.text.len() as u32, line, group, after, act });
    }
    tab.guard = guard(&tab);
    tab
}

/// The file's guard: its first directive an `#ifndef X` with no text before it, of one arm, its
/// `#endif` the last directive with no text after it, every directive between inside it.
fn guard(tab: &Tab) -> Option<u32> {
    if tab.dirs.len() < 2 {
        return None;
    }
    let (first, last) = (&tab.dirs[0], &tab.dirs[tab.dirs.len() - 1]);
    let one = first.kind == Kind::Ifndef && first.text_before == 0 && tab.chains.first().is_some_and(|c| c.len() == 1);
    let closed = last.kind == Kind::Endif && last.after == 0 && last.text_before as usize == tab.text.len();
    let inside = tab.dirs[1..tab.dirs.len() - 1].iter().all(|d| d.group != 0 && d.after != 0);
    let name = first.line.first().filter(|n| n.kind == P::Ident)?;
    (one && closed && inside).then_some(name.sym)
}

/// The rest of the directive's line.
fn rest_of(lx: &mut PpLexer<'static>, pp: &mut Pp, file: u32) -> Vec<Tk> {
    let mut v = Vec::new();
    while let Some(t) = lx.next_on_line() {
        v.push(pp.file_tok(t, file));
    }
    v
}

/// A `#pragma`'s act, by its first word, and a push's or pop's `( "name" )` (`Pp::pragma_text`'s).
fn pragma(pp: &mut Pp, text: &[u8]) -> Act {
    let mut lx = PpLexer::new(text, pp.cfg.opts);
    let t = lx.next();
    if t.kind != P::Ident {
        return Act::Nothing;
    }
    let word = &text[t.start as usize..t.end as usize];
    if word == b"once" {
        return Act::Once;
    }
    if word != b"push_macro" && word != b"pop_macro" {
        return Act::Nothing;
    }
    let (lp, st) = (lx.next(), lx.next());
    if lp.kind != P::Punct || lp.punct != rerail_c_lean::T::LParen || st.kind != P::Str {
        return Act::Nothing;
    }
    let name = super::super::destringize(&text[st.start as usize..st.end as usize]);
    let sym = pp.syms.get(&name);
    if word == b"push_macro" { Act::Push(sym) } else { Act::Pop(sym) }
}
