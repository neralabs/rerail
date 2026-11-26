# 0370: The combine's tail decides from kept part summaries, and rewrites only the changed part

Status: superseded by 0393 (audited 2026-11-23). Proposed (2026-11-17). 0345's stage 3, after 0368 (the kept link).
Work queue Q18.

## Intro

After 0368, an edit of one policy's body costs about 2.6 s CPU on ep2. The combine is now the largest phase
(0.54 s). Most of it is the tail that runs over the whole combined program after the 317 programs are read: fact
spaces 0.077 s, hiding's rounds 0.066 s, the sources on request 0.042 s, narrowing the requests, merging identical
relations, and the projections and helpers' retyping. Each runs over about 5,800 rules on every edit, though only
one policy's rules changed.

These passes cannot simply run per part, as stage 2's did. Each decides something across programs:

- **Fact spaces** (0091) puts relations into congruence classes over the whole program, and from those decides
  which library relations are one copy for the run and which a policy changed (its own). It then guards each
  narrower policy's rules.
- **Hiding** (0058) is a fixpoint: what is known of a relation's location variables flows from the rules that make
  it to the rules that read it, across programs.
- **The sources on request** (0174) make one request relation from every reader of an attribute, in every program.
- **Merging identical relations** names relations by their content, across programs.

A part alone gives none of these the answer the whole gives.

## Decision

Each tail pass is split in two, as 0281's analyses and rule passes are split:

1. **Its decision**, made over the whole, but from a summary per part rather than from every rule. A part's summary
   is what the decision reads of it:
   - for fact spaces, the congruence class of each relation the part makes, its own set, and its scope;
   - for hiding, the facts the part's rules carry out of the relations they make, given the facts carried into
     what they read;
   - for the sources on request, the attributes the part's rules read;
   - for the merges, the content key of each relation the part makes.
2. **Its rewrite**, per rule, from the decision: the guards, the hidden bounds and copies, the readers' requests,
   the renames.

The summaries are kept per part, as stored facts under the part's key (0103), beside the last tail's output (as
0368 keeps the last link). On an edit:

3. **Only the changed parts' summaries are made again.** The others are read from the store.
4. **The decisions are made again from the summaries.** This is cheap: the decision works over 318 summaries, not
   5,800 rules. Hiding's fixpoint runs over relations' facts, starting from the kept ones, and stops where they do
   not change.
5. **If every decision that touches an unchanged part is as it was, the tail's output is spliced** in the same way
   0368 splices the link: the unchanged parts' rules copied from the last output, and the changed parts' rules
   rewritten from the decisions. Otherwise the tail runs whole and its output and summaries are kept.

## What is to be shown before it is taken

- That each pass's rewrite of a rule reads only its decision and the rule itself (no other rule). Fact spaces'
  guards and the merges' renames do. Hiding's copies for guests need checking: a copy is made per rule, from that
  rule's facts.
- That the summaries are small next to the rules: one row per relation a part makes, about 20 per policy.
- That a debug build checks a spliced tail against a whole one (as stage 1's link is checked against the tree).

## Expected

At most the tail's cost on an edit (about 0.25 s of 0.54 s in `combine`), less the changed part's summaries and
the decisions over summaries. What stays is reading the 317 programs' images and texts, which 0246 §6 already
keeps. Its per-program cost (about 0.3 ms each) is measured separately.
