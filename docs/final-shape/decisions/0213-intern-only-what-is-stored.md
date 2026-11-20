# 0213: Intern only what is stored: text for the nodes rules read, no atom for a text only looked up

Status: built (2026-11-14, commit a07f7ede4). Follows decision 0110 (nodes get `text` only where rules read it).

## Intro

An atom is a string the store keeps for good: in the atoms table, in every frame that names it, and in the warm
runs that read it back. PostHog's store held 12.7 M atoms (789 MB). One file of it held 3,617, and most of them were
never needed:

- **1,892 node texts**: statements, comments and docstrings. No rule read them. One text read that the demand could
  not narrow made `text` for every node.
- **Splits and probes**: every prefix and suffix of every dotted name (`@dot` run backwards), and every `@dot` or
  text function result made only to look it up (`workspace_module(@dot(m, x))`).

## Decision

**Text is made only for nodes a rule can read.** The text demand (`syntax::text_kind_demand`) narrows three more
forms. Each widened every node before:

```
kind(l, "generic_type"), child(l, v, 0), text(v, "Literal")   // only the nodes whose text is "Literal"
kind(n, "dotted_name"), child(n, c, 0), text(c, t)            // only the children of dotted_name nodes
field(i, "module_name", mn), child(mn, dn, 1), text(dn, m)    // only the children of nodes in that field
```

The last form also covers a parent narrowed by a matcher capture, which is what `kind` and `field` bodies are
rewritten into (`rr_ast_3_1(i, mn)`).

**The native join interns no text it only looks up** (`setfix::probes`). A `@dot`, a text function's result, or a
split's part can be one whose slot no head reads and whose only readers are positive atoms reading it as a key.
Such a text is found as interning would find it (`Atoms::get`), and none is made. A miss ends the branch, because no
row can hold a string that is no atom. A text that nothing reads gets no atom and no lookup, but the branch goes on.
`match`'s unread captures are of this kind.

## Consequences

- The one-file store holds 1,984 atoms with 57 KB of strings (was 3,617 and 127 KB), with the same findings.
- The findings JSON is built two pieces at a time (`rr/lower/findings.rs`), and each partial string was an atom.
  Closed by decision 0228: a text read only as text is held in the branch's scratch, as the device holds one in its
  arena, so no n-ary form was needed.
- A negated read (`!text(v, "self")`) narrows and demands as a positive one does. It once demanded nothing: a policy
  whose only read of some nodes' text was negated made no row for them, and the negation held of every one, `self`
  included; beside a policy that read those texts, it found what it should. A negation of a constant within the bound
  needs no text past the bound (a longer text is not that constant); one that is not, of a node nothing narrows, makes
  every node's text at any length, since no twin (0109) stands in for a negation.
