//! A host file's tape, read in place by its embedders: the named nodes an embedder asks for (a
//! node's kind, its bytes, its named children, its parent and named siblings), answered from the
//! tape's columns (`TapeImage`), with nothing decoded or rebuilt. Whichever parser read the host,
//! tree-sitter or a lean parser, its tape is the same, so an embedder builds the same program from
//! either.
//!
//! The tape stores every named node; the tokens it leaves out (`tape::tokens`) are anonymous, and
//! no embedder reads one. A node's children are the nodes after it, each one past the last's
//! subtree (the `size` column); its parent is found from the root by the same steps.

use crate::code_rule_dsl::tape::{Grammar, SENTINEL, TAIL_FROM_START, TapeImage};
use std::ops::Range;

/// The host's tape and its grammar.
#[derive(Clone, Copy)]
pub struct Host<'t> {
    image: &'t TapeImage,
    grammar: &'t Grammar,
}

impl<'t> Host<'t> {
    pub fn new(image: &'t TapeImage, grammar: &'t Grammar) -> Self {
        Self { image, grammar }
    }

    /// The tape's root.
    pub fn root(self) -> Option<HostNode<'t>> {
        (self.image.node_count() > 0).then_some(HostNode { host: self, index: 0 })
    }
}

/// A node of the host's tape, by its preorder.
#[derive(Clone, Copy)]
pub struct HostNode<'t> {
    host: Host<'t>,
    index: u32,
}

impl PartialEq for HostNode<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && std::ptr::eq(self.host.image, other.host.image)
    }
}

impl Eq for HostNode<'_> {}

impl std::fmt::Debug for HostNode<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {:?}", self.kind(), self.byte_range())
    }
}

impl<'t> HostNode<'t> {
    fn at(self, index: u32) -> Self {
        Self { index, ..self }
    }

    fn i(self) -> usize {
        self.index as usize
    }

    /// One past its subtree's last node.
    fn after(self) -> u32 {
        self.index + self.host.image.size_at(self.i())
    }

    /// Its kind's name, and whether it is named: the contextual alias where it has one.
    fn name(self) -> (&'t str, bool) {
        let image = self.host.image;
        let code = &image.codes[usize::from(image.code_at(self.i()))];
        if code.alias != SENTINEL {
            let (name, named) = &image.kind_aliases[usize::from(code.alias)];
            return (name.as_str(), *named);
        }
        let kind = code.kf() & 0x3ff;
        (self.host.grammar.kind_name(kind).unwrap_or("ERROR"), self.host.grammar.kind_is_named(kind))
    }

    pub fn kind(self) -> &'t str {
        self.name().0
    }

    pub fn is_named(self) -> bool {
        self.name().1
    }

    pub fn start_byte(self) -> usize {
        self.host.image.start_at(self.i()) as usize
    }

    pub fn end_byte(self) -> usize {
        let image = self.host.image;
        let from = if image.widths[2] & TAIL_FROM_START != 0 { self.i() } else { self.after() as usize - 1 };
        (image.start_at(from) + image.tail_at(self.i())) as usize
    }

    pub fn byte_range(self) -> Range<usize> {
        self.start_byte()..self.end_byte()
    }

    /// Its children, named or not, as the tape stores them.
    pub fn children(self) -> impl Iterator<Item = HostNode<'t>> + 't {
        let end = self.after();
        let mut next = self.index + 1;
        std::iter::from_fn(move || {
            (next < end).then(|| {
                let child = self.at(next);
                next = child.after();
                child
            })
        })
    }

    pub fn named_children(self) -> impl Iterator<Item = HostNode<'t>> + 't {
        self.children().filter(|child| child.is_named())
    }

    pub fn named_child(self, nth: usize) -> Option<HostNode<'t>> {
        self.named_children().nth(nth)
    }

    /// Its parent: the root's descendant holding it, stepped down to from the root.
    pub fn parent(self) -> Option<HostNode<'t>> {
        let mut at = self.host.root()?;
        if at == self {
            return None;
        }
        loop {
            let child = at.children().find(|child| child.index <= self.index && self.index < child.after())?;
            if child == self {
                return Some(at);
            }
            at = child;
        }
    }

    pub fn prev_named_sibling(self) -> Option<HostNode<'t>> {
        self.parent()?.named_children().take_while(|sibling| sibling.index < self.index).last()
    }

    pub fn next_named_sibling(self) -> Option<HostNode<'t>> {
        self.parent()?.named_children().find(|sibling| sibling.index > self.index)
    }
}

/// A host file's tape from tree-sitter, owned: the reader's input in tests.
#[cfg(any(test, feature = "test-hooks"))]
pub struct Tape {
    image: TapeImage,
    grammar: std::sync::Arc<Grammar>,
}

#[cfg(any(test, feature = "test-hooks"))]
impl Tape {
    /// The tape of `source`, read by `dialect`'s tree-sitter grammar.
    pub fn of(dialect: crate::code_rule_dsl::language::GrammarDialect, source: &str) -> Self {
        use ast_grep_language::LanguageExt;
        let grammar = Grammar::new(dialect).expect("grammar");
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&dialect.get_ts_language()).expect("tree-sitter grammar");
        let tree = parser.parse(source, None).expect("tree");
        let image = TapeImage::encode(tree.root_node(), &grammar, source, &|| false).expect("encodes").primary;
        Self { image, grammar }
    }

    pub fn root(&self) -> HostNode<'_> {
        Host::new(&self.image, &self.grammar).root().expect("a root")
    }
}
