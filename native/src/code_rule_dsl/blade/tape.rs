//! A Blade template's tape: Rerail's port of Forte parses it (`blade::forte`), and its tree, its syntax errors (as
//! ERROR leaves) and the program its embedded PHP derives (`blade::view`) are encoded here. The engine registers
//! `Forte` as Blade's tree parser (`tape::TreeParser`).
use std::ops::Range;

use ast_grep_core::source::SgNode;

use crate::code_rule_dsl::blade::forte::tree::{BladeNode, BladeTree, parse_tree};
use crate::code_rule_dsl::blade::view::{ReadError, View};
use crate::code_rule_dsl::language::Via;
use crate::code_rule_dsl::tape::{
    DerivedImage, ERROR_CODE, FIELD_LIMIT, FileImage, Grammar, KIND_LIMIT, TapeImage, TemplateError, TreeOut, TreeParser, Visit,
    checked,
};

/// Blade's tree parser.
pub struct Forte;

impl TreeParser for Forte {
    fn parse(&self, grammar: &Grammar, source: &str, budget: &dyn Fn() -> bool) -> Option<Result<FileImage, TemplateError>> {
        Some(encode_template(grammar, source, &budget))
    }

    /// Its compiled view, from the template's tree (`blade::view`).
    fn embeds_itself(&self) -> bool {
        true
    }

    fn kinds(&self) -> Option<crate::code_rule_dsl::tape::OwnTable> {
        use crate::code_rule_dsl::blade::forte::tree::{FIELD_NAMES, FIRST_ANONYMOUS, KIND_NAMES};
        Some((KIND_NAMES, FIRST_ANONYMOUS, FIELD_NAMES))
    }

    /// `comment` in a Blade template names its comments of every kind (`<!-- -->`, `{{-- --}}`,
    /// `<!--[if ...]>` and `<!...>`).
    fn kind_names(&self, kind: &str) -> Option<&'static [&'static str]> {
        (kind == "comment").then_some(&["comment", "blade_comment", "conditional_comment", "bogus_comment"][..])
    }

    /// Blade's patterns are Forte trees (`blade::forte::tree`), whose kinds are numbered as Blade's tapes number them.
    fn build_pattern(
        &self,
        builder: &ast_grep_core::matcher::PatternBuilder,
    ) -> Option<Result<ast_grep_core::matcher::Pattern, ast_grep_core::matcher::PatternError>> {
        use ast_grep_core::language::Language;
        Some(crate::code_rule_dsl::blade::forte::tree::BladeLang.build_pattern(builder))
    }
}

/// Where a Blade template's syntax error (`BladeTree::error_ranges`) sits in its tree as an ERROR
/// leaf: under the deepest node whose span holds it with bytes on both sides (a zero-width error
/// strictly inside), after the children that end at or before its start. Siblings then still end
/// in byte order. The node's id and the child index the leaf goes before.
pub fn error_place(root: BladeNode<'_>, error: &Range<usize>) -> (usize, usize) {
    let mut node = root;
    loop {
        let inner = node.children().find(|child| {
            let range = child.range();
            range.start <= error.start
                && error.end <= range.end
                && range.start < error.end
                && error.start < range.end
        });
        match inner {
            Some(child) => node = child,
            None => break,
        }
    }
    let before = node
        .children()
        .take_while(|child| child.range().end <= error.start)
        .count();
    (node.node_id(), before)
}

/// A Blade template's tree (`blade::forte::tree`) written to `out` in preorder, its kinds and
/// fields the tree's, and its syntax errors `errors` (as Blade reads them, `View::read`) as ERROR
/// leaves (`error_place`).
fn encode_blade_tree(
    out: &mut dyn TreeOut,
    tree: &BladeTree,
    errors: &[Range<usize>],
    budget: &impl Fn() -> bool,
) -> Result<(), String> {
    enum Item<'t> {
        Node(BladeNode<'t>),
        Error(Range<usize>),
        /// The end of the subtree of the node opened before its children.
        Close,
    }
    let mut errors = errors
        .iter()
        .map(|error| {
            let (node, before) = error_place(tree.root(), error);
            (node, before, error.clone())
        })
        .collect::<Vec<_>>();
    errors.sort_by_key(|(node, before, error)| (*node, *before, error.start, error.end));
    let mut stack = vec![(Item::Node(tree.root()), None)];
    let mut visited = 0usize;
    while let Some((item, parent)) = stack.pop() {
        if visited.is_multiple_of(256) {
            checked(budget)?;
        }
        visited += 1;
        let node = match item {
            Item::Error(range) => {
                let start = u32::try_from(range.start).map_err(|_| "node span overflow")?;
                let end = u32::try_from(range.end).map_err(|_| "node span overflow")?;
                end.checked_sub(start).ok_or("inverted node span")?;
                out.open(Visit { kf: ERROR_CODE, start, end, extra: false, error: true, missing: false, kind_alias: None }, parent, &[])?;
                out.close();
                continue;
            }
            Item::Close => {
                out.close();
                continue;
            }
            Item::Node(node) => node,
        };
        let field = node.field_id();
        let code = node.kind_id();
        if code == ERROR_CODE
            || usize::from(code) >= KIND_LIMIT
            || usize::from(field) >= FIELD_LIMIT
        {
            return Err(format!(
                "Blade kind {code} or field {field} exceeds the tape's codes"
            ));
        }
        let range = node.range();
        let start = u32::try_from(range.start).map_err(|_| "node span overflow")?;
        let end = u32::try_from(range.end).map_err(|_| "node span overflow")?;
        end.checked_sub(start).ok_or("inverted node span")?;
        let index = out.open(Visit { kf: code | (field << 10), start, end, extra: false, error: false, missing: false, kind_alias: None }, parent, &[])?;
        let id = node.node_id();
        let here = errors.partition_point(|(node, ..)| *node < id)
            ..errors.partition_point(|(node, ..)| *node <= id);
        let mut placed = errors[here].iter().peekable();
        let mut items = Vec::with_capacity(node.children().len() + placed.len());
        for (position, child) in node.children().enumerate() {
            while let Some((_, _, error)) = placed.next_if(|(_, before, _)| *before <= position) {
                items.push(Item::Error(error.clone()));
            }
            items.push(Item::Node(child));
        }
        items.extend(placed.map(|(_, _, error)| Item::Error(error.clone())));
        stack.push((Item::Close, None));
        stack.extend(items.into_iter().rev().map(|item| (item, Some(index))));
    }
    Ok(())
}

/// The documents of a Blade template, parsed with Rerail's port of Forte (`encode_blade`).
pub fn encode_template(
    grammar: &Grammar,
    source: &str,
    budget: &impl Fn() -> bool,
) -> Result<FileImage, TemplateError> {
    checked(budget)?;
    let tree = parse_tree(source.as_bytes()).map_err(|error| TemplateError::Parser(error.message()))?;
    encode_blade(&tree, grammar, source, budget)
}

/// The documents of a Blade template from its Forte tree (`tree::parse_tree`): the
/// template's tree, its syntax errors as ERROR leaves, and the document its embedded PHP
/// derives (`blade::view`, the `compiled-view` embedding).
pub(in crate::code_rule_dsl) fn encode_blade(
    tree: &BladeTree,
    grammar: &Grammar,
    source: &str,
    budget: &impl Fn() -> bool,
) -> Result<FileImage, TemplateError> {
    check_blade_input(grammar, source, budget)?;
    // The template's syntax errors are ERROR leaves where Blade reads them (`View::read`).
    let read = View::read(tree, source, budget).map_err(|error| match error {
        ReadError::Parser(error) => TemplateError::Parser(error.message()),
        ReadError::Interrupted => TemplateError::Encode("tape operation interrupted".into()),
    })?;
    let image = TapeImage::walked(grammar, source, source.len() / 8 + 16, budget, |out| encode_blade_tree(out, tree, &read.syntax_errors, budget))?;
    let image = FileImage::single(image, source);
    with_compiled_view(image, read.view, grammar, source, budget)
}

/// Why `encode_blade` can't encode `source` with `grammar`: the budget ran out, the
/// grammar isn't one Forte parses, or the source is past the tape's range.
fn check_blade_input(
    grammar: &Grammar,
    source: &str,
    budget: &impl Fn() -> bool,
) -> Result<(), TemplateError> {
    checked(budget)?;
    if !crate::code_rule_dsl::language::adapter::parsed_by_forte(grammar.lang) {
        return Err("only a Blade template is encoded from a Forte tree"
            .to_owned()
            .into());
    }
    if source.len() > u32::MAX as usize {
        return Err("source exceeds tape range".to_owned().into());
    }
    Ok(())
}

/// `image` with the document its template's embedded PHP derives, where it embeds any
/// (`blade::view`, the `compiled-view` embedding).
fn with_compiled_view(
    mut image: FileImage,
    view: Option<View>,
    grammar: &Grammar,
    source: &str,
    budget: &impl Fn() -> bool,
) -> Result<FileImage, TemplateError> {
    if let Some(view) = view {
        image.derived.push(DerivedImage::encode(
            Via::CompiledView,
            view.into_program(),
            0,
            grammar,
            source,
            budget,
        )?);
    }
    Ok(image)
}
