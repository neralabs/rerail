//! Rules that cross between the documents of a file that embeds code (`embed`): `embedded`, from a
//! node of the file's own tree to the nodes of its embedded program that stand for bytes it spans,
//! and `host` on a program's node, to the innermost node of the file's own tree that spans the
//! bytes it stands for. Each document is matched in an environment of its own, as a `parse`'s
//! tree is: `embedded` exports the captures its rule binds to the host's environment (their text,
//! and the file bytes they stand for); `host` binds nothing, and runs in the host's environment as
//! the enclosing `embedded` began (a fresh one in a clause over the program, `ast.embedded`) with
//! the program rule's captures exported into it.

use super::AstMatcher;
use super::each::{self, HostRanges};
use super::exported::{export, exported_span, exported_text, is_span_variable};
use crate::code_rule_dsl::language::{GrammarDialect, Via};
use crate::code_rule_dsl::origin::Origin;
use crate::code_rule_dsl::tape::{TapeDoc, tape_doc};
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
use ast_grep_core::{AstGrep, Doc, Matcher, Node};
use std::borrow::Cow;
use std::cell::RefCell;
use std::ops::Range;
use std::sync::Arc;

/// `embedded`: a rule over the program a file embeds through `via`, compiled for each dialect the
/// program may be written in.
#[derive(Clone)]
pub(crate) struct EmbeddedTest {
    pub(crate) via: Via,
    pub(crate) rules: Vec<(GrammarDialect, AstMatcher)>,
    /// The captures the rule binds, which the host's environment receives.
    pub(crate) exports: Vec<String>,
}

/// A program's captures as the host reads them: name, text, file range.
type Exports = [(String, String, Range<usize>)];

/// The host a program's `host` rule runs against: of the innermost `embedded` being matched over
/// that program, which runs a host rule on the host's node spanning a file range.
struct Hook {
    run: *const (dyn Fn(&AstMatcher, &Exports, Range<usize>) -> bool + 'static),
    /// The program document the `embedded` reads (`TapeDoc::same`).
    program: TapeDoc,
}

thread_local! {
    /// The `embedded` tests being matched whose rules hold `host`, innermost last.
    static HOOKS: RefCell<Vec<Hook>> = const { RefCell::new(Vec::new()) };
}

/// An `embedded` test's host while its rule is matched; dropping it restores the enclosing one.
struct Hosted;

impl Hosted {
    fn enter<'f>(
        run: &'f (dyn Fn(&AstMatcher, &Exports, Range<usize>) -> bool + 'f),
        program: TapeDoc,
    ) -> Self {
        // SAFETY: the pointer is read only while this guard lives, and the guard is dropped before
        // `run`, which the caller declares first; the lifetime is erased only to store it.
        let run: *const (dyn Fn(&AstMatcher, &Exports, Range<usize>) -> bool + 'f) = run;
        let run = unsafe {
            std::mem::transmute::<
                *const (dyn Fn(&AstMatcher, &Exports, Range<usize>) -> bool + 'f),
                *const (dyn Fn(&AstMatcher, &Exports, Range<usize>) -> bool + 'static),
            >(run)
        };
        HOOKS.with_borrow_mut(|hooks| hooks.push(Hook { run, program }));
        Self
    }
}

impl Drop for Hosted {
    fn drop(&mut self) {
        HOOKS.with_borrow_mut(|hooks| hooks.pop());
    }
}

/// The innermost node under `root` whose range holds `at` (a node of width preferred to an empty
/// one at the same place).
fn innermost<'r, D: Doc>(root: Node<'r, D>, at: &Range<usize>) -> Node<'r, D> {
    let mut node = root;
    loop {
        let inner = node.children().find(|child| {
            let range = child.range();
            range.start <= at.start && at.end <= range.end && (at.is_empty() || !range.is_empty())
        });
        match inner {
            Some(child) => node = child,
            None => return node,
        }
    }
}

/// The captures bound so far in a program's environment `env` as its host reads them: each node
/// capture's text and the file bytes it stands for, and each capture a nested `parse` exported,
/// its span mapped out of the program.
fn program_exports<D: Doc>(
    env: &MetaVarEnv<'_, D>,
    program: &TapeDoc,
) -> Vec<(String, String, Range<usize>)> {
    let mut exports = Vec::new();
    for variable in env.get_matched_variables() {
        let MetaVariable::Capture(name, _) = variable else {
            continue;
        };
        if is_span_variable(&name) {
            continue;
        }
        if let Some(found) = env.get_match(&name)
            && let Some(range) = program.of_node(found.node_id(), found.range())
        {
            exports.push((name.clone(), found.text().into_owned(), range));
        } else if let (Some(text), Some(span)) =
            (exported_text(env, &name), exported_span(env, &name))
            && let Some(range) = program.of_range(span)
        {
            exports.push((name.clone(), text, range));
        }
    }
    exports
}

impl EmbeddedTest {
    pub(super) fn match_node<'t, D: Doc>(
        &self,
        node: Node<'t, D>,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> Option<Node<'t, D>> {
        let doc = tape_doc(&node)?;
        let guests = doc.guests()?;
        let guest = guests.iter().find(|guest| guest.via == self.via)?;
        let dialect = *guest.doc.get_lang();
        let (_, rule) = self.rules.iter().find(|(known, _)| *known == dialect)?;
        let span = node.range();
        let program = AstGrep::doc(guest.doc.clone());
        // A `host` rule, in the rule or a utility it reaches, runs on the host's node spanning the
        // program node's bytes, in the host's environment as the test began, with the program
        // rule's captures.
        let before = env.as_ref().clone();
        let host_root = node.root().root();
        let run = |rule: &AstMatcher, exports: &Exports, at: Range<usize>| {
            let mut host = Cow::Borrowed(&before);
            for (name, text, range) in exports {
                export(&mut host, name, text, range.clone());
            }
            let target = innermost(host_root.clone(), &at);
            each::suspended(|| rule.match_node_with_env(target, &mut host).is_some())
        };
        let _hosted = Hosted::enter(&run, guest.doc.clone());
        // `violation.each` reads the program's bindings at the file bytes they stand for, as the
        // host's environment receives them, and passes over a candidate binding one it reported.
        let _hosts = each::active().then(|| HostRanges::enter(Arc::new(guest.doc.clone())));
        // The program's nodes standing for bytes within the node's, in preorder: a node's bytes
        // hold its descendants', so a subtree whose bytes miss the node's is passed over.
        let mut stack = vec![program.root()];
        while let Some(candidate) = stack.pop() {
            let Some(at) = guest.doc.of_node(candidate.node_id(), candidate.range()) else {
                continue;
            };
            if at.start > span.end || span.start > at.end {
                continue;
            }
            if span.start <= at.start && at.end <= span.end {
                let mut inner = Cow::Owned(MetaVarEnv::new());
                if rule
                    .match_node_with_env(candidate.clone(), &mut inner)
                    .is_some()
                    && !(each::active() && each::excluded(&inner))
                {
                    for (name, text, range) in program_exports(&inner, &guest.doc) {
                        if self.exports.contains(&name) {
                            export(env, &name, &text, range);
                        }
                    }
                    return Some(node);
                }
            }
            let children = candidate.children().collect::<Vec<_>>();
            stack.extend(children.into_iter().rev());
        }
        None
    }

    /// Its rules, one per dialect.
    pub(crate) fn rules(&self) -> impl Iterator<Item = &AstMatcher> {
        self.rules.iter().map(|(_, rule)| rule)
    }
}

/// A program node's `host` rule (not inside `parse`): whether `rule` matches the host's innermost
/// node spanning the bytes `node` stands for. `None` where `node` is no program's.
pub(super) fn document_host<D: Doc>(
    rule: &AstMatcher,
    node: &Node<'_, D>,
    env: &MetaVarEnv<'_, D>,
) -> Option<bool> {
    let doc = tape_doc(node).filter(|doc| doc.is_guest())?;
    let at = doc.of_node(node.node_id(), node.range())?;
    let exports = program_exports(env, doc);
    let hooked = HOOKS.with_borrow(|hooks| {
        hooks
            .iter()
            .rev()
            .find(|hook| hook.program.same(doc))
            .map(|hook| hook.run)
    });
    if let Some(run) = hooked {
        // SAFETY: `Hosted` keeps the closure alive while it is on the stack (see `Hosted::enter`).
        let run = unsafe { &*run };
        return Some(run(rule, &exports, at));
    }
    let host = AstGrep::doc(doc.host()?.clone());
    let target = innermost(host.root(), &at);
    let mut env = Cow::Owned(MetaVarEnv::new());
    for (name, text, range) in &exports {
        export(&mut env, name, text, range.clone());
    }
    Some(each::suspended(|| {
        rule.match_node_with_env(target, &mut env).is_some()
    }))
}

#[cfg(test)]
mod tests;
