//! Rerail's own suppression comment, `rerail-ignore RULE-ID[, RULE-ID ...] [-- reason]`: always
//! honoured, for every code rule, whatever the repository's `suppressions` setting (which governs
//! only other linters' comments, `suppress`). A directive covers a node of the file's tree:
//!
//! - before the file's first code (only comments, white space, a shebang and the family's
//!   preamble tokens, PHP's `<?php`, before it): the whole file;
//! - on its own line (leading): the next named sibling after it in its parent, comments passed
//!   over (before the first statement of a body that starts there, Python's, that statement);
//!   nothing where none follows (the end of a block);
//! - after code on the line where it starts (trailing): the nearest preceding named sibling, when
//!   only comments stand between them and it ends on that line; past a `,` or `;` ending a list
//!   element or class field on that line, the elements before it on the line, from the one
//!   holding the line's first code byte; otherwise the smallest ancestor of the comment holding
//!   that byte (never the file's root).
//!
//! A violation is silenced when the byte its anchor names falls in a range covered for its rule's
//! id: the anchor's start, or for an end anchor the witness's last byte (the point where a witness
//! ends is where the next node may start).
//!
//! A file that embeds code (`embed`) is read document by document: its own tree's comments, then
//! each embedded program's, whose covers are the file bytes the covered nodes stand for. Only the
//! file's own tree's comments may cover the whole file. Each document's family names the node
//! kinds it is read by (`Adapter::COMMENT_KINDS`, `PREAMBLE_KINDS` and `BODY_KINDS`).

use crate::code_rule_dsl::embed::segments::FileRanges;
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use crate::code_rule_dsl::origin::InFile;
use crate::code_rule_dsl::tape::{TapeDoc, tape_doc};
use ast_grep_core::{AstGrep, Doc, Node};
use std::ops::Range;
use crate::code_rule_dsl::language::adapter::pack as language_pack;

/// The directive's name.
const DIRECTIVE: &str = "rerail-ignore";

/// A byte range covered for the rule ids a directive names.
#[derive(Debug, PartialEq, Eq)]
struct Cover {
    /// Exclusive of its end, as the covered node's; the whole file's runs to `usize::MAX`, past an
    /// anchor at the end of the file.
    range: Range<usize>,
    rules: Vec<String>,
}

/// A file's `rerail-ignore` directives, as the ranges they cover.
#[derive(Debug, Default)]
pub(crate) struct RerailIgnores {
    covers: Vec<Cover>,
}

impl RerailIgnores {
    /// Reads every comment of the file whose own tree `root` is the root of (`source` is its
    /// text), then each embedded program's (`embed`), each by its own family's kinds; every cover
    /// is a range of the file.
    pub(crate) fn read<D: Doc>(root: &Node<'_, D>, source: &str) -> Self {
        let file = InFile::of(root).expect("a file's own tree lies in its file");
        let kinds = Kinds::of(file.dialect());
        let mut ignores = Self::read_tree(root, source, &file, kinds, true);
        let guests = tape_doc(root).and_then(TapeDoc::guests);
        for guest in guests.iter().flat_map(|guests| guests.iter()) {
            let program = AstGrep::doc(guest.doc.clone());
            let file = InFile::tape(&guest.doc);
            let kinds = Kinds::of(*guest.doc.get_lang());
            let read =
                Self::read_tree(&program.root(), guest.doc.get_source(), &file, kinds, false);
            ignores.covers.extend(read.covers);
        }
        ignores
    }

    /// Reads every comment of the tree `root` is the root of, whose family's kinds are `kinds`;
    /// `source` is its whole text, `file` the file it lies in. A comment before the tree's first
    /// code covers the whole file only where `whole`.
    fn read_tree<D: Doc>(
        root: &Node<'_, D>,
        source: &str,
        file: &InFile<'_>,
        kinds: Kinds,
        whole: bool,
    ) -> Self {
        let mut covers = Vec::new();
        // A file that never spells the directive holds none: no walk.
        if !source.contains(DIRECTIVE) {
            return Self { covers };
        }
        let mut first = if whole { None } else { Some(0) };
        for comment in root.dfs().filter(|node| kinds.comment(node)) {
            let range = comment.range();
            let rules = rule_ids(&source[range.clone()]);
            if rules.is_empty() {
                continue;
            }
            let first = *first.get_or_insert_with(|| first_code(root, source, kinds));
            let covered = if range.end <= first {
                Some(FileRanges::one(0..usize::MAX))
            } else {
                covered(&comment, source, file, kinds)
            };
            // A covered node of a program standing for several file ranges covers each.
            for range in covered.iter().flat_map(FileRanges::iter) {
                covers.push(Cover {
                    range: range.clone(),
                    rules: rules.iter().copied().map(str::to_owned).collect(),
                });
            }
        }
        Self { covers }
    }

    /// Each directive's covered range and the code rules it names.
    pub(crate) fn covers(&self) -> impl Iterator<Item = (&Range<usize>, &[String])> {
        self.covers.iter().map(|cover| (&cover.range, cover.rules.as_slice()))
    }

    /// Whether any directive of the file names `rule`.
    pub(crate) fn names(&self, rule: &str) -> bool {
        self.covers
            .iter()
            .any(|cover| cover.rules.iter().any(|name| name == rule))
    }

    /// Whether a violation of `rule` whose anchor names byte `at` is silenced.
    pub(crate) fn ignores(&self, rule: &str, at: usize) -> bool {
        self.covers
            .iter()
            .any(|cover| cover.range.contains(&at) && cover.rules.iter().any(|name| name == rule))
    }
}

/// The node kinds a family's trees are read by (`Adapter::COMMENT_KINDS`, `PREAMBLE_KINDS` and
/// `BODY_KINDS`).
#[derive(Clone, Copy)]
struct Kinds {
    comments: &'static [&'static str],
    preamble: &'static [&'static str],
    bodies: &'static [&'static str],
}

impl Kinds {
    /// The kinds of the family of `dialect`.
    fn of(dialect: GrammarDialect) -> Self {
        { let pack = language_pack::pack(Family::of(dialect)); Self {
            comments: pack.comment_kinds(),
            preamble: pack.preamble_kinds(),
            bodies: pack.body_kinds(),
        } }
    }

    fn comment<D: Doc>(self, node: &Node<'_, D>) -> bool {
        self.comments.contains(&node.kind().as_ref())
    }
}

/// The rule ids a comment's directive names, from the whole comment text (delimiters included):
/// after the comment's opener and any `*` and white space, `rerail-ignore`, then white space and
/// ids separated by commas or white space, up to the end of the line or a ` -- ` reason. None
/// where the comment is no directive, or one naming no rule.
fn rule_ids(comment: &str) -> Vec<&str> {
    let mut text = comment.trim_end();
    if let Some(inner) = ["--}}", "-->", "*/"]
        .into_iter()
        .find_map(|closer| text.strip_suffix(closer))
    {
        text = inner;
    }
    // A script's HTML-like comment opens with `<!--`, or with `-->` at the start of a line.
    let Some(inner) = ["{{--", "<!--", "-->", "//", "/*", "#", "--"]
        .into_iter()
        .find_map(|opener| text.strip_prefix(opener))
    else {
        return Vec::new();
    };
    let text = inner.trim_start_matches(|c: char| c == '*' || c.is_whitespace());
    let Some(rest) = text.strip_prefix(DIRECTIVE) else {
        return Vec::new();
    };
    // `rerail-ignored` or `rerail-ignore-next` is another word, no directive.
    if !rest.starts_with(|c: char| c.is_whitespace()) {
        return Vec::new();
    }
    let line = rest.split(['\n', '\r']).next().unwrap_or_default();
    before_reason(line)
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|id| !id.is_empty())
        .collect()
}

/// `list` up to a reason: two or more dashes with white space before them and white space (or the
/// end) after, as an ESLint directive's description starts.
fn before_reason(list: &str) -> &str {
    let bytes = list.as_bytes();
    for (at, byte) in bytes.iter().enumerate() {
        if !byte.is_ascii_whitespace() {
            continue;
        }
        let dashes = bytes[at + 1..]
            .iter()
            .take_while(|byte| **byte == b'-')
            .count();
        let after = at + 1 + dashes;
        if dashes >= 2 && bytes.get(after).is_none_or(u8::is_ascii_whitespace) {
            return &list[..at];
        }
    }
    list
}

/// The first byte of the file's code: the earliest leaf outside comments that is no shebang (`#!`
/// line), preamble token (`kinds`: PHP's open tag) or white space. The file's length where it has
/// none.
fn first_code<D: Doc>(root: &Node<'_, D>, source: &str, kinds: Kinds) -> usize {
    let mut first = source.len();
    let mut stack = vec![root.clone()];
    while let Some(node) = stack.pop() {
        let range = node.range();
        // A comment's tokens (Blade's `{{--` and its text) are no code; nor is anything after
        // code already found.
        if kinds.comment(&node) || range.start >= first {
            continue;
        }
        if !node.is_leaf() {
            stack.extend(node.children());
            continue;
        }
        let text = &source[range.clone()];
        let shebang = range.start == 0 && text.starts_with("#!") && !text.trim_end().contains('\n');
        if !text.trim().is_empty() && !shebang && !kinds.preamble.contains(&node.kind().as_ref()) {
            first = range.start;
        }
    }
    first
}

/// Whether a node is code a directive can cover: named, no comment, not white space alone.
fn is_code<D: Doc>(node: &Node<'_, D>, source: &str, kinds: Kinds) -> bool {
    node.is_named()
        && !kinds.comment(node)
        && !source[node.range()].trim().is_empty()
        // An embedded program's node its embedder wrote whole (a Blade directive's marker) is no
        // code of the file's.
        && InFile::of(node).is_none_or(|file| file.copies(node.range()))
}

/// The file range a directive comment past its tree's first code covers; the tree's nodes and
/// lines are placed in `file` (an embedded program's comment on a line of its own is one in the
/// file, whatever its embedder wrote before it).
fn covered<D: Doc>(
    comment: &Node<'_, D>,
    source: &str,
    file: &InFile<'_>,
    kinds: Kinds,
) -> Option<FileRanges> {
    let start = file.node(comment).start;
    let lines = file.lines();
    let line = lines.line(start);
    let line_start = lines.start(line);
    let before = file.source().get(line_start..start)?;
    if before.trim().is_empty() {
        // Leading: the next code among its siblings. A body that starts at its first statement
        // (`kinds.bodies`: Python's `block`) has a comment before that statement beside it, not
        // in it: the comment leads that statement.
        let mut next = comment.next();
        while let Some(node) = next {
            if is_code(&node, source, kinds) {
                let first = kinds
                    .bodies
                    .contains(&node.kind().as_ref())
                    .then(|| node.children().find(|child| is_code(child, source, kinds)))
                    .flatten()
                    .filter(|child| child.range().start == node.range().start);
                return Some(file.place(&first.unwrap_or(node)));
            }
            next = node.next();
        }
        return None;
    }
    // Trailing: the code it directly follows, when that ends on its line. Past a `,` or `;` that
    // ends there (a list element's or class field's), the elements before it on the line, from
    // the one holding the line's first code byte: in `m(n, //` that is no element but the call
    // opened on the line.
    let code = line_start + (before.len() - before.trim_start().len());
    // Whether the walk passed a separator, and the end of the element that separator closes.
    let mut separated = false;
    let mut end = None;
    let mut previous = comment.prev();
    while let Some(node) = previous {
        previous = node.prev();
        if kinds.comment(&node) || source[node.range()].trim().is_empty() {
            continue;
        }
        let at = file.node(&node);
        if lines.line(at.end.saturating_sub(1).max(at.start)) != line {
            break;
        }
        if !node.is_named() {
            if matches!(node.kind().as_ref(), "," | ";") {
                separated = true;
                continue;
            }
            break;
        }
        if !separated {
            return Some(file.place(&node));
        }
        // The elements on the comment's line, from the one holding its first code byte.
        let end = *end.get_or_insert(at.end);
        if at.start <= code {
            return Some(FileRanges::one(at.start..end));
        }
    }
    // Otherwise the smallest construct holding the line's code: the comment sits inside it.
    let mut ancestor = comment.parent();
    while let Some(node) = ancestor {
        let parent = node.parent();
        parent.as_ref()?;
        if file.node(&node).start <= code {
            return Some(file.place(&node));
        }
        ancestor = parent;
    }
    None
}

#[cfg(test)]
mod tests;
