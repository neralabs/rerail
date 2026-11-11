//! Typed call candidates over core posting ranges. Exact semantics and findings remain consumers.
use super::*;
use std::collections::BTreeSet;
#[derive(Default, serde::Serialize)]
pub(crate) struct Work {
    pub calls: usize,
    pub posting_calls: usize,
    pub context_ranges: usize,
    pub contexts: usize,
    pub screens: usize,
    pub emitted: usize,
    pub direct: usize,
    pub receivers: usize,
}
/// Production uses async-body ranges where available and exact context checks for every selected call.
pub(crate) fn collect(
    g: &Grammar,
    f: &Facts,
    context_kind: u32,
    members: &BTreeSet<String>,
    admits: impl FnMut(u32) -> bool,
) -> (Vec<[u32; 6]>, Work) {
    collect_shaped(
        g,
        f,
        context_kind,
        members,
        if context_kind == 2 {
            "context_ranges"
        } else {
            "context_first"
        },
        admits,
    )
}
#[cfg(any(test, feature = "test-hooks"))]
pub(crate) fn experiment(
    g: &Grammar,
    f: &Facts,
    context_kind: u32,
    members: &BTreeSet<String>,
    mode: &str,
    admits: impl FnMut(u32) -> bool,
) -> (Vec<[u32; 6]>, Work) {
    collect_shaped(g, f, context_kind, members, mode, admits)
}
/// Rows: call, report, reference, receiver, flags, execution owner, all core IDs.
fn collect_shaped(
    g: &Grammar,
    f: &Facts,
    context_kind: u32,
    members: &BTreeSet<String>,
    mode: &str,
    mut admits: impl FnMut(u32) -> bool,
) -> (Vec<[u32; 6]>, Work) {
    let mut work = Work::default();
    let mut out = Vec::new();
    work.posting_calls = g.calls.iter().map(|&kind| f.of_kind(kind).len()).sum();
    let mut classify = |report: u32, work: &mut Work| {
        let reference = matches!(g.role(f, report), Role::Name | Role::Attribute)
            .then_some(report)
            .unwrap_or(NONE);
        let direct = reference != NONE && {
            work.screens += 1;
            admits(reference)
        };
        let receiver = if g.role(f, report) == Role::Attribute {
            field(f, report, g.attribute)
                .filter(|&name| {
                    members.contains(crate::code_rule_dsl::identifier::nfkc(f.text(name)).as_ref())
                })
                .and_then(|_| field(f, report, g.object))
                .unwrap_or(NONE)
        } else {
            NONE
        };
        (
            reference,
            receiver,
            u32::from(direct) | (u32::from(receiver != NONE) << 1),
        )
    };
    if mode == "structural" {
        let rows = structure(g, f);
        work.calls = rows.site.len();
        work.contexts = rows.context.len();
        for ([report, call, _], [c, owner, kind]) in rows.site.into_iter().zip(rows.context) {
            assert_eq!(c, call);
            if kind != context_kind {
                continue;
            }
            let (reference, receiver, flags) = classify(report, &mut work);
            if flags != 0 {
                out.push([call, report, reference, receiver, flags, owner]);
            }
        }
    } else {
        assert!(matches!(
            mode,
            "context_first" | "screen_first" | "context_ranges"
        ));
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        if mode == "context_ranges" {
            assert_eq!(
                context_kind, 2,
                "the experiment selects async-function body ranges"
            );
            for (code, role) in g.roles.iter().enumerate() {
                if *role != Role::Function {
                    continue;
                }
                for &function in f.of_kind(kind_atom(g.tag, code as u16)) {
                    if f.children(function)
                        .iter()
                        .any(|&c| g.role(f, c) == Role::Async)
                    {
                        if let Some(body) = field(f, function, g.body) {
                            ranges.push((body, f.subtree_end[body as usize]));
                        }
                    }
                }
            }
            ranges.sort_unstable();
            let mut merged: Vec<(u32, u32)> = Vec::new();
            for (lo, hi) in ranges {
                if let Some(last) = merged.last_mut().filter(|r| lo <= r.1) {
                    last.1 = last.1.max(hi);
                } else {
                    merged.push((lo, hi));
                }
            }
            ranges = merged;
            work.context_ranges = ranges.len();
        }
        let mut selected = Vec::new();
        for &kind in &g.calls {
            let calls = f.of_kind(kind);
            if mode == "context_ranges" {
                for &(lo, hi) in &ranges {
                    let begin = calls.partition_point(|&n| n < lo);
                    let end = calls.partition_point(|&n| n < hi);
                    selected.extend_from_slice(&calls[begin..end]);
                }
            } else {
                selected.extend_from_slice(calls);
            }
        }
        for call in selected {
            work.calls += 1;
            let Some(callee) = field(f, call, g.function) else {
                continue;
            };
            let report = unparen(g, f, callee);
            let early = if mode == "screen_first" {
                let value = classify(report, &mut work);
                if value.2 == 0 {
                    continue;
                }
                Some(value)
            } else {
                None
            };
            work.contexts += 1;
            let (owner, kind) = context(g, f, call);
            if kind != context_kind {
                continue;
            }
            let (reference, receiver, flags) = early.unwrap_or_else(|| classify(report, &mut work));
            if flags != 0 {
                out.push([call, report, reference, receiver, flags, owner]);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    work.emitted = out.len();
    work.direct = out.iter().filter(|r| r[4] & 1 != 0).count();
    work.receivers = out.iter().filter(|r| r[4] & 2 != 0).count();
    (out, work)
}

/// Preserve the reference's Known / NotApplicable / Unknown distinction on core facts.
pub(crate) fn eligibility(
    g: &Grammar,
    f: &Facts,
    requested: u32,
) -> crate::code_rule_dsl::facts::producers::python::python_facts::Fact<
    crate::code_rule_dsl::facts::producers::python::python_facts::QualificationChain,
> {
    use crate::code_rule_dsl::facts::producers::python::python_facts::{
        Fact, QualificationChain, Reason,
    };
    let bad = |n: u32| f.flags[n as usize] & 12 != 0;
    let damaged = |n: u32| {
        if (n..f.subtree_end[n as usize]).any(&bad) {
            return true;
        }
        let mut at = Some(n);
        while let Some(n) = at {
            if bad(n) {
                return true;
            }
            at = f.parent(n);
        }
        false
    };
    let unwrap = |mut n: u32| -> Result<u32, Reason> {
        loop {
            if bad(n) {
                return Err(Reason::SyntaxError);
            }
            if g.role(f, n) != Role::Parens {
                return Ok(n);
            }
            let mut parts = f
                .children(n)
                .iter()
                .copied()
                .filter(|&c| f.named(c) && g.role(f, c) != Role::Comment);
            match (parts.next(), parts.next()) {
                (Some(inner), None) => n = inner,
                _ => {
                    return Err(if damaged(n) {
                        Reason::SyntaxError
                    } else {
                        Reason::UnsupportedExpression
                    });
                }
            }
        }
    };
    let chain = (|| -> Result<QualificationChain, Reason> {
        let mut head = unwrap(requested)?;
        let mut attributes = Vec::new();
        while g.role(f, head) == Role::Attribute {
            let attribute = field(f, head, g.attribute).ok_or(Reason::SyntaxError)?;
            if g.role(f, attribute) != Role::Name || bad(attribute) {
                return Err(Reason::SyntaxError);
            }
            attributes.push(
                crate::code_rule_dsl::identifier::python_name(f.text(attribute).into())
                    .into_owned(),
            );
            head = unwrap(field(f, head, g.object).ok_or(Reason::SyntaxError)?)?;
        }
        if g.role(f, head) != Role::Name {
            return Err(if damaged(requested) {
                Reason::SyntaxError
            } else {
                Reason::UnsupportedExpression
            });
        }
        let mut at = Some(head);
        while let Some(n) = at {
            if bad(n) {
                return Err(Reason::SyntaxError);
            }
            at = f.parent(n);
        }
        attributes.reverse();
        Ok(QualificationChain {
            head: crate::code_rule_dsl::identifier::python_name(f.text(head).into()).into_owned(),
            attributes,
        })
    })();
    match chain {
        Ok(c) => Fact::Known(c),
        Err(Reason::UnsupportedExpression) => Fact::NotApplicable(Reason::UnsupportedExpression),
        Err(r) => Fact::Unknown(r),
    }
}

#[test]
fn core_eligibility_equals_reference_on_known_inapplicable_and_broken_expressions() {
    use crate::code_rule_dsl::{
        facts::{core, producers::python::python_facts},
        language::{GrammarDialect, tape_tag},
        lines::{LineTable, Terminators},
        tape::{Grammar as TapeGrammar, TapeDoc, TapeImage},
    };
    use ast_grep_core::{AstGrep, tree_sitter::StrDoc};
    use std::sync::Arc;
    for source in [
        "factory().open()\nhandlers['open']()\nPath('f').open()\n(real.name)()\n",
        "broken = (\n",
        "async def f():\n    (thing.).open()\n",
        "async def f():\n    pkg.ｍｅｔｈｏｄ()\n    ((pkg.name))()\n",
    ] {
        let parsed = StrDoc::new(source, GrammarDialect::Python);
        let grammar = TapeGrammar::new(GrammarDialect::Python).unwrap();
        let image =
            TapeImage::encode(parsed.tree.root_node(), &grammar, source, &|| false).unwrap();
        let root = AstGrep::doc(
            TapeDoc::restore(Arc::new(source.into()), grammar, &image, &|| false).unwrap(),
        );
        let facts = core::from_tape(
            root.root().get_doc(),
            Arc::new(LineTable::new(
                source,
                Terminators::of(GrammarDialect::Python),
            )),
        )
        .unwrap();
        let g = Grammar::new(
            tape_tag(GrammarDialect::Python).unwrap(),
            &GrammarDialect::Python.kinds(),
        );
        // Every expression/node, including malformed ancestors, checks conservative handling.
        for node in root.root().dfs() {
            assert_eq!(
                eligibility(&g, &facts, node.node_id() as u32),
                python_facts::qualification_chain(&node),
                "{source}: {}",
                node.text()
            );
        }
    }
}
