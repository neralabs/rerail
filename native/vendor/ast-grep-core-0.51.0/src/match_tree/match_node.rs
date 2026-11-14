use super::Aggregator;
use super::strictness::{MatchOneNode, MatchStrictness};
use crate::matcher::{PatternNode, kind_utils};
use crate::meta_var::MetaVariable;
use crate::{Doc, Node};
use std::iter::Peekable;

pub(super) fn match_root_node_impl<'tree, D: Doc>(
  goal: &PatternNode,
  candidate: &Node<'tree, D>,
  agg: &mut impl Aggregator<'tree, D>,
  strictness: &MatchStrictness,
) -> MatchOneNode {
  // Smart matching ignores comments inside structured patterns, but a root
  // metavariable represents the candidate itself and must still bind extras.
  if let PatternNode::MetaVar { meta_var } = goal
    && matches!(strictness, MatchStrictness::Smart)
  {
    return match agg.match_meta_var(meta_var, candidate) {
      Some(()) => MatchOneNode::MatchedBoth,
      None => MatchOneNode::NoMatch,
    };
  }
  match_node_impl(goal, candidate, agg, strictness)
}

pub(super) fn match_node_impl<'tree, D: Doc>(
  goal: &PatternNode,
  candidate: &Node<'tree, D>,
  agg: &mut impl Aggregator<'tree, D>,
  strictness: &MatchStrictness,
) -> MatchOneNode {
  use PatternNode as P;
  match &goal {
    // leaf = without named children
    P::Terminal {
      text,
      kind_id,
      is_named,
    } => match strictness.match_terminal(*is_named, text, *kind_id, candidate) {
      MatchOneNode::MatchedBoth => {
        if agg.match_terminal(candidate).is_some() {
          MatchOneNode::MatchedBoth
        } else {
          MatchOneNode::NoMatch
        }
      }
      c => c,
    },
    P::MetaVar { meta_var, .. } => {
      if strictness.should_skip_cand_for_metavar(candidate) {
        return MatchOneNode::SkipCandidate;
      }
      match agg.match_meta_var(meta_var, candidate) {
        Some(()) => MatchOneNode::MatchedBoth,
        None => MatchOneNode::NoMatch, // TODO: this may be wrong
      }
    }
    P::Internal {
      kind_id, children, ..
    } => {
      let kind_matched = strictness.should_skip_kind()
        || kind_utils::are_kinds_matching(*kind_id, candidate.kind_id());
      if !kind_matched {
        return MatchOneNode::NoMatch;
      }
      match match_nodes_impl_recursive(children, candidate, agg, strictness) {
        Some(()) => MatchOneNode::MatchedBoth,
        None => MatchOneNode::NoMatch,
      }
    }
  }
}

fn match_nodes_impl_recursive<'tree, D: Doc>(
  goals: &[PatternNode],
  parent: &Node<'tree, D>,
  agg: &mut impl Aggregator<'tree, D>,
  strictness: &MatchStrictness,
) -> Option<()> {
  let mut goal_children = goals.iter().peekable();
  let mut cand_children = parent.children().peekable();
  // Rerail (RER-131): the candidate the last goal matched, for the end of children.
  let mut last_matched = None;
  if cand_children.peek().is_none() {
    // Rerail (RER-131): a node with no children (a tape leaves out the tokens its grammar
    // implies) matches goals that are ellipses, each bound to nothing, and the end of children.
    for goal in goals {
      if goal.is_children_end() {
        continue;
      }
      let name = try_get_ellipsis_mode(goal).ok()?;
      agg.match_ellipsis(name.as_deref(), Vec::new(), 0)?;
    }
    return Some(());
  }
  loop {
    match may_match_ellipsis_impl(&mut goal_children, &mut cand_children, agg, strictness)? {
      ControlFlow::Return => return Some(()),
      ControlFlow::Continue => continue,
      ControlFlow::Fallthrough => (),
    }
    let Some(flow) = match_single_node_while_skip_trivial(
      &mut goal_children,
      &mut cand_children,
      agg,
      strictness,
    ) else {
      // Rerail (RER-131): the end of children refused a named candidate. Where the tokens the
      // candidate leaves out after the last matched child begin with the goal's, those are the
      // goal's (upstream matches them there) and the rest trails, as after upstream's last goal.
      let ends_there = goal_children.peek().and_then(|goal| goal.children_end_word()).is_some_and(|word| {
        !word.is_empty()
          && cand_children.peek().is_some()
          && last_matched.as_ref().is_some_and(|last: &Node<'tree, D>| implied_after(parent, last, &word))
      });
      return (ends_there && cand_children.all(|n| strictness.should_skip_trailing(&n))).then_some(());
    };
    match flow {
      ControlFlow::Return => return Some(()),
      ControlFlow::Continue => continue,
      ControlFlow::Fallthrough => (),
    }
    let consumed_goal = goal_children.next();
    // if goal runs out, do not proceed cand nodes
    if consumed_goal.is_some() {
      last_matched = cand_children.next();
    }
    if goal_children.peek().is_none() {
      // all goal found
      let has_trailing = cand_children.all(|n| strictness.should_skip_trailing(&n));
      return has_trailing.then_some(());
    }
    if cand_children.peek().is_none() {
      // Rerail (RER-131): the end of children holds where the candidates end.
      return goal_children.all(PatternNode::is_children_end).then_some(());
    }
  }
}

/// Rerail (RER-131): whether the tokens `parent`'s document leaves out right after its child `child`
/// begin with `tokens` (`Doc::implied_after`).
fn implied_after<D: Doc>(parent: &Node<'_, D>, child: &Node<'_, D>, tokens: &[&str]) -> bool {
  let Some(at) = parent.children().position(|n| n.node_id() == child.node_id()) else {
    return false;
  };
  parent.get_doc().implied_after(&parent.inner, at, tokens) == Some(true)
}

enum ControlFlow {
  Continue,
  Fallthrough,
  Return,
}

/// returns None means no match
fn may_match_ellipsis_impl<'p, 't: 'p, D: Doc>(
  goal_children: &mut Peekable<impl Iterator<Item = &'p PatternNode>>,
  cand_children: &mut Peekable<impl Iterator<Item = Node<'t, D>>>,
  agg: &mut impl Aggregator<'t, D>,
  strictness: &MatchStrictness,
) -> Option<ControlFlow> {
  let Some(curr_node) = goal_children.peek() else {
    // in rare case, an internal node's children is empty
    // see https://github.com/ast-grep/ast-grep/issues/1688
    return Some(ControlFlow::Return);
  };
  let Ok(optional_name) = try_get_ellipsis_mode(curr_node) else {
    return Some(ControlFlow::Fallthrough);
  };
  let mut matched = vec![];
  goal_children.next();
  // goal has all matched
  if goal_children.peek().is_none() {
    match_ellipsis(agg, &optional_name, matched, cand_children, 0)?;
    return Some(ControlFlow::Return);
  }
  // skip trivial nodes in goal after ellipsis
  let mut skipped_anonymous = 0;
  while goal_children.peek().unwrap().is_trivial() {
    let goal = goal_children.next().unwrap();
    // Rerail (RER-131): the end-of-children goal stands for the implied tokens after the
    // pattern's last child. A candidate keeps those tokens only where its own gap is not that word
    // (`print(a, b,)` keeps `,` and `)`): the ones it keeps are the goal's, not the ellipsis's.
    let end_word = goal.children_end_word();
    if end_word.is_none() {
      skipped_anonymous += 1;
    }
    if goal_children.peek().is_none() {
      let mut rest: Vec<_> = cand_children.collect();
      for token in end_word.iter().flatten().rev() {
        match rest.last() {
          Some(last) if !last.is_named() && last.text() == *token => {
            rest.pop();
          }
          _ => break,
        }
      }
      match_ellipsis(
        agg,
        &optional_name,
        matched,
        rest.into_iter(),
        skipped_anonymous,
      )?;
      return Some(ControlFlow::Return);
    }
  }
  // if next node is a Ellipsis, consume one candidate node
  if try_get_ellipsis_mode(goal_children.peek().unwrap()).is_ok() {
    matched.push(cand_children.next().unwrap());
    cand_children.peek()?;
    match_ellipsis(
      agg,
      &optional_name,
      matched,
      std::iter::empty(),
      skipped_anonymous,
    )?;
    return Some(ControlFlow::Continue);
  }
  loop {
    // Probe the next goal against a cloned aggregator to find the ellipsis end.
    // This prevents failed metavar probes from leaking bindings into the real env.
    // Leaking them would make a later, genuine bind of the same metavar conflict and fail
    // Note the match_node_impl here matches the peek nodes, the real consumption and
    // env write will be done in `match_single_node_while_skip_trivial`
    // See https://github.com/ast-grep/ast-grep/pull/2670
    let mut probe = agg.clone();
    if matches!(
      match_node_impl(
        goal_children.peek().unwrap(),
        cand_children.peek().unwrap(),
        &mut probe,
        strictness,
      ),
      MatchOneNode::MatchedBoth
    ) {
      // found match non Ellipsis,
      match_ellipsis(
        agg,
        &optional_name,
        matched,
        std::iter::empty(),
        skipped_anonymous,
      )?;
      break Some(ControlFlow::Fallthrough);
    }
    matched.push(cand_children.next().unwrap());
    cand_children.peek()?;
  }
}

fn match_single_node_while_skip_trivial<'p, 't: 'p, D: Doc>(
  goal_children: &mut Peekable<impl Iterator<Item = &'p PatternNode>>,
  cand_children: &mut Peekable<impl Iterator<Item = Node<'t, D>>>,
  agg: &mut impl Aggregator<'t, D>,
  strictness: &MatchStrictness,
) -> Option<ControlFlow> {
  loop {
    let Some(cand) = cand_children.peek() else {
      // if cand runs out, check remaining goal
      // if goal is skippable, it is a match, else a non match
      return strictness
        .should_skip_goal(goal_children)
        .then_some(ControlFlow::Fallthrough);
    };
    // try match goal node with candidate node
    match match_node_impl(goal_children.peek().unwrap(), cand, agg, strictness) {
      MatchOneNode::MatchedBoth => return Some(ControlFlow::Fallthrough),
      MatchOneNode::SkipGoal => {
        goal_children.next();
        if goal_children.peek().is_none() {
          return Some(ControlFlow::Fallthrough);
        }
      }
      MatchOneNode::SkipBoth => {
        cand_children.next();
        goal_children.next();
        if goal_children.peek().is_none() {
          return Some(ControlFlow::Fallthrough);
        }
      }
      // skip trivial node
      MatchOneNode::SkipCandidate => {
        cand_children.next();
      }
      // unmatched significant node
      MatchOneNode::NoMatch => return None,
    }
  }
}

/// Returns Ok if ellipsis pattern is found. If the ellipsis is named, returns it name.
/// If the ellipsis is unnamed, returns None. If it is not ellipsis node, returns Err.
fn try_get_ellipsis_mode(node: &PatternNode) -> Result<Option<String>, ()> {
  let PatternNode::MetaVar { meta_var, .. } = node else {
    return Err(());
  };
  match meta_var {
    MetaVariable::Multiple => Ok(None),
    MetaVariable::MultiCapture(n) => Ok(Some(n.into())),
    _ => Err(()),
  }
}

fn match_ellipsis<'t, D: Doc>(
  agg: &mut impl Aggregator<'t, D>,
  optional_name: &Option<String>,
  mut matched: Vec<Node<'t, D>>,
  cand_children: impl Iterator<Item = Node<'t, D>>,
  skipped_anonymous: usize,
) -> Option<()> {
  matched.extend(cand_children);
  agg.match_ellipsis(optional_name.as_deref(), matched, skipped_anonymous)?;
  Some(())
}

#[cfg(test)]
mod test {
  use super::*;
  use crate::language::Tsx;
  use crate::matcher::KindMatcher;
  use crate::{Matcher, Pattern, Root, meta_var::MetaVarEnv};
  use std::borrow::Cow;
  fn match_tree(p: &str, n: &str, strictness: MatchStrictness) -> MatchOneNode {
    let pattern = Pattern::new(p, Tsx);
    let kind = pattern.potential_kinds().expect("should have kind");
    let kind = KindMatcher::from_id(kind.into_iter().next().expect("should have kind") as u16);
    let n = Root::str(n, Tsx);
    let n = n.root().find(kind).expect("should find");
    let mut env = Cow::Owned(MetaVarEnv::new());
    match_node_impl(&pattern.node, &*n, &mut env, &strictness)
  }
  fn matched(p: &str, n: &str, strictness: MatchStrictness) {
    let ret = match_tree(p, n, strictness);
    assert!(
      matches!(ret, MatchOneNode::MatchedBoth),
      "expect match. pattern: `{p}`, node: `{n}`"
    );
  }
  fn unmatched(p: &str, n: &str, strictness: MatchStrictness) {
    let ret = match_tree(p, n, strictness);
    assert!(
      !matches!(ret, MatchOneNode::MatchedBoth),
      "expect no match. pattern: `{p}`, node: `{n}`"
    );
  }
  use MatchStrictness as M;

  #[test]
  fn test_smart_match() {
    // smart now ignores comments by default
    matched("$A($B)", "foo(/* before */ bar /* after */)", M::Smart);
  }

  #[test]
  fn test_ast_match() {
    matched("import $A from 'lib'", "import A from \"lib\"", M::Ast);
    unmatched("$A(bar)", "foo(/* A*/bar)", M::Ast);
    matched("$A(bar)", "foo(bar)", M::Ast);
    unmatched("$A(bar)", "foo(bar, baz)", M::Ast);
    matched("print($A,)", "print(123)", M::Ast);
    matched("print($$$A,b,$$$C)", "print(b)", M::Ast);
    matched("print($$$A,b,$$$C)", "print(a, b)", M::Ast);
    matched("print($$$A,b,$$$C)", "print(a, b, c)", M::Ast);
    matched("print($$$A,b,$$$C)", "print(a, b, c,)", M::Ast);
  }

  #[test]
  fn test_relaxed_match() {
    matched("import $A from 'lib'", "import A from \"lib\"", M::Relaxed);
    matched("$A(bar)", "foo(/* A*/bar)", M::Relaxed);
    // fix https://github.com/ast-grep/ast-grep/issues/1848
    matched(
      "import { foo } from 'bar'",
      "import { foo, } from 'bar'",
      M::Relaxed,
    );
    matched("foo($A, $B)", "foo(1/*test*/, 2/*test*/)", M::Relaxed);
    unmatched(
      "import { foo } from 'bar'",
      "import { foo, bar, baz } from 'bar'",
      M::Relaxed,
    );
    unmatched(
      "import { foo } from 'bar'",
      "import { foo, bar } from 'bar'",
      M::Relaxed,
    );
  }

  #[test]
  fn test_cst_match() {
    unmatched("import $A from 'lib'", "import A from \"lib\"", M::Cst);
    unmatched("$A(bar)", "foo(/* A*/bar)", M::Cst);
    unmatched("print($A,)", "print(123)", M::Cst);
  }

  #[test]
  fn test_signature_match() {
    matched(
      "import $A from 'lib'",
      "import A from \"lib\"",
      M::Signature,
    );
    matched("$A(bar)", "foo(/* A*/bar)", M::Signature);
  }

  #[test]
  fn test_template_match() {
    matched("$A = $B", "a = 123", M::Template);
  }

  // Rerail (RER-131): a document that leaves out every anonymous token, as a tape leaves out the
  // tokens its grammar implies, and says which it left out after a child (`Doc::implied_after`).
  #[derive(Clone)]
  struct Tokenless(crate::tree_sitter::StrDoc<Tsx>);

  #[derive(Clone, Copy)]
  struct TokenlessNode<'r>(tree_sitter::Node<'r>);

  fn kept(node: &tree_sitter::Node<'_>) -> bool {
    node.is_named() || node.is_extra()
  }

  impl<'r> crate::source::SgNode<'r> for TokenlessNode<'r> {
    fn parent(&self) -> Option<Self> {
      self.0.parent().map(TokenlessNode)
    }
    fn children(&self) -> impl ExactSizeIterator<Item = Self> {
      let mut cursor = self.0.walk();
      let children: Vec<_> = self.0.children(&mut cursor).filter(kept).map(TokenlessNode).collect();
      children.into_iter()
    }
    fn kind(&self) -> Cow<'_, str> {
      Cow::Borrowed(self.0.kind())
    }
    fn kind_id(&self) -> u16 {
      self.0.kind_id()
    }
    fn node_id(&self) -> usize {
      self.0.id()
    }
    fn range(&self) -> std::ops::Range<usize> {
      self.0.start_byte()..self.0.end_byte()
    }
    fn start_pos(&self) -> crate::Position {
      crate::source::SgNode::start_pos(&self.0)
    }
    fn end_pos(&self) -> crate::Position {
      crate::source::SgNode::end_pos(&self.0)
    }
    fn is_named(&self) -> bool {
      self.0.is_named()
    }
    fn is_extra(&self) -> bool {
      self.0.is_extra()
    }
    fn field(&self, name: &str) -> Option<Self> {
      self.0.child_by_field_name(name).map(TokenlessNode)
    }
    fn field_children(&self, field_id: Option<u16>) -> impl Iterator<Item = Self> {
      crate::source::SgNode::field_children(&self.0, field_id).filter(kept).map(TokenlessNode)
    }
    fn child_by_field_id(&self, field_id: u16) -> Option<Self> {
      self.0.child_by_field_id(field_id).map(TokenlessNode)
    }
  }

  impl Doc for Tokenless {
    type Source = String;
    type Lang = Tsx;
    type Node<'r> = TokenlessNode<'r>;
    fn get_lang(&self) -> &Tsx {
      &self.0.lang
    }
    fn get_source(&self) -> &String {
      &self.0.src
    }
    fn do_edit(&mut self, _edit: &crate::source::Edit<String>) -> Result<(), String> {
      Err("a test document is not edited".into())
    }
    fn root_node(&self) -> TokenlessNode<'_> {
      TokenlessNode(self.0.tree.root_node())
    }
    fn get_node_text<'a>(&'a self, node: &TokenlessNode<'a>) -> Cow<'a, str> {
      self.0.get_node_text(&node.0)
    }
    fn implied_after<'a>(&'a self, parent: &TokenlessNode<'a>, child: usize, tokens: &[&str]) -> Option<bool> {
      use crate::source::SgNode;
      let mut next = parent.children().nth(child)?.0.next_sibling();
      let mut left_out = Vec::new();
      while let Some(node) = next.filter(|node| !node.is_named() || node.is_extra()) {
        if !node.is_extra() {
          left_out.push(node.utf8_text(self.0.src.as_bytes()).ok()?);
        }
        next = node.next_sibling();
      }
      Some(left_out.starts_with(tokens))
    }
  }

  fn tokenless_matched(goal: &PatternNode, source: &str, kind: &str, strictness: MatchStrictness) -> bool {
    let root = Root::doc(Tokenless(crate::tree_sitter::StrDoc::new(source, Tsx)));
    let candidate = root.root().find(KindMatcher::new(kind, Tsx)).expect("should find");
    let mut env = Cow::Owned(MetaVarEnv::new());
    matches!(
      match_node_impl(goal, &candidate, &mut env, &strictness),
      MatchOneNode::MatchedBoth
    )
  }

  #[test]
  fn test_children_end_meets_implied_tokens() {
    use crate::Language;
    let var = |name: &str| PatternNode::MetaVar {
      meta_var: MetaVariable::Capture(name.into(), false),
    };
    // `case $V:` as a tape keeps it: `case` and `:` left out, `:` after the last child.
    let case = PatternNode::Internal {
      kind_id: Tsx.kind_to_id("switch_case"),
      children: vec![var("V"), PatternNode::children_end(vec![":".into()])],
    };
    let target = "switch (x) { case 2: break; }";
    // The `:` left out after `2` is the goal's; `break;` trails, as after upstream's `:`.
    assert!(tokenless_matched(&case, target, "switch_case", M::Smart));
    assert!(!tokenless_matched(&case, target, "switch_case", M::Ast));
    // `f($A)`: `(`, `)` left out, `)` after the last argument.
    let call = PatternNode::Internal {
      kind_id: Tsx.kind_to_id("call_expression"),
      children: vec![
        PatternNode::Terminal {
          text: "f".into(),
          is_named: true,
          kind_id: Tsx.kind_to_id("identifier"),
        },
        PatternNode::Internal {
          kind_id: Tsx.kind_to_id("arguments"),
          children: vec![var("A"), PatternNode::children_end(vec![")".into()])],
        },
      ],
    };
    assert!(tokenless_matched(&call, "f(a)", "call_expression", M::Smart));
    // The gap after `a` is `,`, not `)`: `b` is no trailing node, as upstream refuses it.
    assert!(!tokenless_matched(&call, "f(a, b)", "call_expression", M::Smart));
    // A document that leaves no token out cannot say: the end of children refuses as before.
    let root = Root::str(target, Tsx);
    let candidate = root.root().find(KindMatcher::new("switch_case", Tsx)).expect("should find");
    let mut env = Cow::Owned(MetaVarEnv::new());
    assert!(!matches!(
      match_node_impl(&case, &candidate, &mut env, &M::Smart),
      MatchOneNode::MatchedBoth
    ));
  }
}
