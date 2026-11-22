//! A Vue single-file component's code as vue-eslint-parser reads it, each piece a program of its own (decision 0597):
//! a `<script>`, and each expression, statement list, parameter list and v-for part of the template. Built on demand
//! (the registry's `demand`): a rule asks for a piece at its place (`parsed(v, "vue-expression", g)`,
//! `lib/vue/code.rerail`), and the run builds the file's pieces of that kind, never at the file's load, so the
//! component's view (`script-block`) stays the only program a file has to every other reader.
//!
//! | `via` | the program's text | its host node |
//! |---|---|---|
//! | `vue-script` | a `<script>`'s text: TypeScript (`lang` none, `js` or `ts`), TSX (`tsx`, `jsx`); another `lang`: no program | its `raw_text` |
//! | `vue-expression` | `(E)`: a `{{ E }}`, a directive's value (but v-on's, v-slot's and v-for's), a dynamic argument `:[E]` | the `raw_text`, the value, the argument's inner value |
//! | `vue-statements` | `S`: a v-on value, as written | the value |
//! | `vue-params` | `(P)=>0`: v-slot, `#x`, slot-scope, scope (`template::Directive`'s) | the value |
//! | `vue-for-alias` | `const [A]=0`: v-for's alias, its own parentheses stripped, holes kept (`const [v, , i]=0`); `let [x]=0` reads as an assignment to `let[x]` | the value |
//! | `vue-for-source` | `(L)`: v-for's list (compiler-core's `parseFor`); a value it does not split is `vue-expression` | the value |
//!
//! The code is copied byte for byte, so its nodes stand at the file's bytes; the wrappers are written, each over the
//! empty range at the code's edge it stands beside. A blank value is no program. A rule's kinds name one grammar, so
//! the code is TypeScript but where it holds JSX: a script without `lang` or with `js` too, and the template's code
//! whatever the scripts' (one grammar per `via`). TypeScript reads the JavaScript a component holds as JavaScript
//! does; rrlab parses those as JavaScript.

use super::host::HostNode;
use super::segments::SegmentMap;
use super::{Embedder, Program};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::vue::template::{self, Arg, Directive};
use std::ops::Range;

/// The kind of piece an embedder builds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Piece {
    Script,
    Expression,
    Statements,
    Params,
    ForAlias,
    ForSource,
}

/// The embedder of one kind of piece (`Via::VueScript`, `Via::VueExpression`, …).
pub struct VueCode(pub Piece);

pub static SCRIPT: VueCode = VueCode(Piece::Script);
pub static EXPRESSION: VueCode = VueCode(Piece::Expression);
pub static STATEMENTS: VueCode = VueCode(Piece::Statements);
pub static PARAMS: VueCode = VueCode(Piece::Params);
pub static FOR_ALIAS: VueCode = VueCode(Piece::ForAlias);
pub static FOR_SOURCE: VueCode = VueCode(Piece::ForSource);

/// One piece found: its kind, the code's bytes, and the host node's bytes it stands at.
struct Found {
    piece: Piece,
    code: Range<usize>,
    at: Range<usize>,
    dialect: GrammarDialect,
}

/// The dialect a `<script>`'s `lang` names; `None` for one no grammar reads (`coffee`).
fn script_dialect(lang: Option<&str>) -> Option<GrammarDialect> {
    match lang.map(str::trim) {
        None | Some("" | "js" | "ts") => Some(GrammarDialect::TypeScript),
        Some("tsx" | "jsx") => Some(GrammarDialect::Tsx),
        Some(_) => None,
    }
}

/// `r` less its blanks at both ends.
fn trimmed(source: &str, r: Range<usize>) -> Range<usize> {
    let s = &source[r.clone()];
    let start = r.start + (s.len() - s.trim_start().len());
    let end = r.end - (s.len() - s.trim_end().len());
    start..end.max(start)
}

/// The value of start tag `start`'s attribute `name`.
fn attribute<'s>(start: HostNode<'_>, source: &'s str, name: &str) -> Option<&'s str> {
    start.named_children().filter(|c| c.kind() == "attribute").find_map(|a| {
        let n = a.named_child(0)?;
        if &source[n.byte_range()] != name {
            return None;
        }
        Some(match a.named_child(1) {
            Some(v) if v.kind() == "quoted_attribute_value" => v.named_child(0).map_or("", |v| &source[v.byte_range()]),
            Some(v) => &source[v.byte_range()],
            None => "",
        })
    })
}

/// Every piece of the component under `root`, in source order.
fn pieces(root: HostNode<'_>, source: &str) -> Vec<Found> {
    let ts = GrammarDialect::TypeScript;
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        let children: Vec<_> = n.named_children().collect();
        match n.kind() {
            "script_element" => {
                let lang = children.iter().find(|c| c.kind() == "start_tag").and_then(|t| attribute(*t, source, "lang"));
                if let (Some(dialect), Some(raw)) = (script_dialect(lang), children.iter().find(|c| c.kind() == "raw_text")) {
                    out.push(Found { piece: Piece::Script, code: raw.byte_range(), at: raw.byte_range(), dialect });
                }
                continue;
            }
            "interpolation" => {
                if let Some(raw) = children.iter().find(|c| c.kind() == "raw_text") {
                    out.push(Found { piece: Piece::Expression, code: raw.byte_range(), at: raw.byte_range(), dialect: ts });
                }
            }
            "attribute" | "directive_attribute" => {
                if let Some(d) = Directive::read(n, source) {
                    if let Arg::Dynamic(inner) = &d.arg
                        && let Some(iv) = n.named_children().find(|c| c.kind() == "dynamic_directive_value").and_then(|c| c.named_children().find(|x| x.byte_range() == *inner))
                    {
                        out.push(Found { piece: Piece::Expression, code: inner.clone(), at: iv.byte_range(), dialect: ts });
                    }
                    if let Some(value) = d.value.clone() {
                        let piece = match d.name {
                            "on" => Piece::Statements,
                            "slot" | "slot-scope" | "scope" => Piece::Params,
                            "for" => Piece::ForAlias,
                            _ => Piece::Expression,
                        };
                        if piece != Piece::ForAlias {
                            out.push(Found { piece, code: value.clone(), at: value, dialect: ts });
                        } else if let Some(f) = template::parse_for(source, value.clone()) {
                            let mut alias = trimmed(source, f.alias.clone());
                            if f.parenthesized {
                                alias = alias.start + 1..alias.end - 1;
                            }
                            out.push(Found { piece: Piece::ForAlias, code: alias, at: value.clone(), dialect: ts });
                            out.push(Found { piece: Piece::ForSource, code: trimmed(source, f.source.clone()), at: value, dialect: ts });
                        } else {
                            out.push(Found { piece: Piece::Expression, code: value.clone(), at: value, dialect: ts });
                        }
                    }
                }
                continue;
            }
            _ => {}
        }
        stack.extend(children.into_iter().rev());
    }
    out
}

/// Piece `f`'s program: its code between its wrappers, the code copied and each wrapper written at its edge.
fn program(f: &Found, source: &str, budget: &dyn Fn() -> bool) -> Option<Program> {
    let code = &source[f.code.clone()];
    if code.trim().is_empty() && f.piece != Piece::Script {
        return None;
    }
    let (open, close) = match f.piece {
        Piece::Script | Piece::Statements => ("", ""),
        Piece::Expression | Piece::ForSource => ("(", ")"),
        Piece::Params => ("(", ")=>0"),
        Piece::ForAlias => ("const [", "]=0"),
    };
    let text = format!("{open}{code}{close}");
    let mut segments = SegmentMap::new();
    if !open.is_empty() {
        segments.push(0, open.len(), f.code.start..f.code.start, false, false);
    }
    segments.push(open.len(), open.len() + code.len(), f.code.clone(), true, false);
    if !close.is_empty() {
        segments.push(open.len() + code.len(), text.len(), f.code.end..f.code.end, false, false);
    }
    segments.index();
    let parse = super::Parse::read(f.dialect, &text, budget)?;
    Some(Program { dialect: f.dialect, parse, text, segments, host_errors: Vec::new(), at: Some(f.at.clone()) })
}

impl Embedder for VueCode {
    fn build(&self, root: HostNode<'_>, source: &str, budget: &dyn Fn() -> bool) -> Option<Program> {
        self.build_all(root, source, budget).into_iter().next()
    }

    fn build_all(&self, root: HostNode<'_>, source: &str, budget: &dyn Fn() -> bool) -> Vec<Program> {
        pieces(root, source).iter().filter(|f| f.piece == self.0).filter_map(|f| program(f, source, budget)).collect()
    }

    fn per_place(&self) -> bool {
        true
    }
}
