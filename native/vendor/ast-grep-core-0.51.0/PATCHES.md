# ast-grep-core 0.51.0, patched (RER-131)

crates.io's ast-grep-core 0.51.0, with one addition for tapes that leave out the tokens their
grammar implies (`native/src/code_rule_dsl/tape/tokens.rs`). Every change is marked `Rerail (RER-131)`.

- **The end-of-children goal** (`kind_utils::CHILDREN_END`, `PatternNode::children_end`). A pattern
  is parsed into a tape like any document, so its implied tokens are left out too. Under `Smart`
  strictness a pattern's tokens are anchors: a named candidate cannot be skipped past one. Only
  the implied tokens after a pattern node's last child carry meaning a tape cannot give back (no
  named child follows), so they become one goal in their place. That goal matches no node, skips a
  token or a comment, refuses a named node, and holds where the candidates end
  (`MatchStrictness::should_skip_goal`). It carries the tokens' texts (`children_end_word`): an
  ellipsis before it leaves the candidate's trailing tokens that are that word (a candidate keeps
  them where its own gap is not the implied word, as `print(a, b,)` keeps `,` and `)`), as
  upstream's ellipsis leaves the tokens the pattern lists after it (`may_match_ellipsis_impl`).
- **`PatternBuilder::build_doc`, `source` and `PatternShape`.** The caller parses the pattern
  itself and says, from the tree it wrote the document from, which nodes end with that goal
  (`convert_node_to_pattern`'s `ends`) and which are single nodes (`is_single_node`, which picks
  the pattern's root: a tape's `(x)` has one child where tree-sitter's had three). `build` is
  `build_doc` with the document's own shape.
- **A node with no children** (all its tokens implied) matches goals that are ellipses, bound to
  nothing, and the end of children (`match_nodes_impl_recursive`). Upstream refuses a candidate
  with no children.
- **`Doc::implied_after`**: whether the tokens a document leaves out right after a parent's child
  begin with given texts; `None` (cannot say) by default, so upstream's documents are unchanged.
  Where the end of children meets a named candidate (`match_nodes_impl_recursive`), the left-out
  tokens after the last matched child that begin with the goal's word are the goal's, and the rest
  trails under the strictness (`should_skip_trailing`), as after upstream's last token goal:
  `case $V:` matches `case 2: break;`. Otherwise it refuses as before: `f($A)` refuses `f(a, b)`,
  whose gap after `a` is `,`.
