//! Markdown's inline programs (`embed::inline_content`, decision 0431 §4) against tree-sitter-md's inline trees. They
//! parse files, so they run where the languages are registered.

use crate::code_rule_dsl::embed::Embedder as _;

/// Each node of `tree` in preorder, named or not: its kind, and its span through `host` (a guest's map).
fn nodes(tree: &tree_sitter::Tree, host: &dyn Fn([u32; 2]) -> [u32; 2]) -> Vec<(String, [u32; 2])> {
    let mut out = Vec::new();
    let mut c = tree.walk();
    loop {
        let n = c.node();
        out.push((n.kind().to_owned(), host([n.start_byte() as u32, n.end_byte() as u32])));
        if c.goto_first_child() {
            continue;
        }
        while !c.goto_next_sibling() {
            if !c.goto_parent() {
                return out;
            }
        }
    }
}

/// The differences, for `md`, between each inline program's tree (its text, `InlineContent`'s, parsed by the inline
/// grammar and mapped back through its segments) and tree-sitter-md's inline tree (`MarkdownParser`: the inline
/// grammar over the node's ranges less its named children after the first). Empty where none differs.
fn guest_differences(md: &str) -> Vec<String> {
    use ast_grep_language::LanguageExt;
    use crate::code_rule_dsl::language::GrammarDialect;
    let tape = crate::code_rule_dsl::embed::host::Tape::of(GrammarDialect::Markdown, md);
    let programs = crate::code_rule_dsl::embed::inline_content::InlineContent.build_all(tape.root(), md, &|| false);
    let block = {
        let mut p = tree_sitter::Parser::new();
        p.set_language(&GrammarDialect::Markdown.get_ts_language()).unwrap();
        p.parse(md, None).unwrap()
    };
    let mut inline = tree_sitter::Parser::new();
    inline.set_language(&GrammarDialect::MarkdownInline.get_ts_language()).unwrap();
    let mut out = Vec::new();
    for program in &programs {
        let ours = inline.parse(&program.text, None).unwrap();
        let ours = nodes(&ours, &|[a, b]| {
            let r = program.segments.bounds(a as usize..b as usize);
            [r.start as u32, r.end as u32]
        });
        // tree-sitter-md's ranges, from its block tree's node at the same place.
        let whole = program.segments.bounds(0..program.text.len());
        let (start, end) = (whole.start, whole.end);
        let node = block.root_node().descendant_for_byte_range(start, end).unwrap();
        let node = std::iter::successors(Some(node), |n| n.parent()).find(|n| n.start_byte() == start && n.end_byte() == end && matches!(n.kind(), "inline" | "pipe_table_cell")).unwrap();
        let mut range = node.range();
        let mut ranges = Vec::new();
        let mut c = node.walk();
        if c.goto_first_child() {
            while c.goto_next_sibling() {
                if !c.node().is_named() {
                    continue;
                }
                let child = c.node().range();
                ranges.push(tree_sitter::Range { start_byte: range.start_byte, start_point: range.start_point, end_byte: child.start_byte, end_point: child.start_point });
                range.start_byte = child.end_byte;
                range.start_point = child.end_point;
            }
        }
        ranges.push(range);
        inline.set_included_ranges(&ranges).unwrap();
        let theirs = inline.parse(md, None).unwrap();
        inline.set_included_ranges(&[]).unwrap();
        let theirs = nodes(&theirs, &|span| span);
        // The roots differ in extent: tree-sitter's inline root spans the whole file's included text; compare below it.
        if ours[1..] != theirs[1..] {
            let at = ours.iter().zip(&theirs).position(|(a, b)| a != b).unwrap_or(ours.len().min(theirs.len()));
            out.push(format!("inline at {start}..{end}: node {at}: ours {:?}, tree-sitter-md's {:?}", ours.get(at), theirs.get(at)));
        }
    }
    out
}

#[test]
fn markdown_inline_guests_are_tree_sitter_mds_inline_trees() {
    let md = "# A *b*\n\n> Some *text* and a [link](u \"t\")\n> on `two` lines, *across\n> lines*.\n\n- item **x**\n  more <b>html</b>\n\n| a | **b** |\n|---|---|\n| `c` | d \\| e |\n";
    assert_eq!(guest_differences(md), Vec::<String>::new());
}

/// The same over the Markdown files `RERAIL_MD_GUEST_FILES` lists.
#[test]
#[ignore = "an oracle over files; needs RERAIL_MD_GUEST_FILES"]
fn markdown_inline_guests_oracle() {
    let list = std::fs::read_to_string(std::env::var("RERAIL_MD_GUEST_FILES").expect("set RERAIL_MD_GUEST_FILES")).unwrap();
    let (mut files, mut differ) = (0, 0);
    for path in list.lines().filter(|l| !l.is_empty()) {
        let Ok(md) = std::fs::read_to_string(path) else { continue };
        files += 1;
        let d = guest_differences(&md);
        if !d.is_empty() {
            differ += 1;
            if differ <= 5 {
                eprintln!("{path}: {} differ; first: {}", d.len(), d[0]);
            }
        }
    }
    eprintln!("{files} files, {differ} with an inline guest that differs");
    assert_eq!(differ, 0);
}
