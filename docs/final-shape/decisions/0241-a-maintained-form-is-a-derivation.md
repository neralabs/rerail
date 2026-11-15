# 0241. A maintained stratum's form is a derivation of the run's version

Status: decided (2026-11-15). Continues decisions 0173 (§7) and 0215 for t36 item 7, step 7.

## Intro

A session's maintained run (decision 0117 step 3) makes a form for each stratum it reaches whose reads changed:
the stratum's rules rewritten to maintain it from deltas and old rows (decision 0077 §3), its relations' deltas,
and its rules for deriving it again (decision 0090 §4). While the form runs, the run reads it as its program: its
rules, its strata, and its fields (inputs, declarations, facts, natives, gates and the rest).

Until now the form was a tree (`maintain::rewrite_with` over a one-stratum program cut from the run's), kept in an
arena for the run's life, and each form was encoded into a whole image of its own. The planner must read every
program it runs from a version, made with the builder (t36 item 7). This record says which kind of version a form
is.

## Decision

A maintained stratum's form is a **derivation** (decision 0173 §7's class for the maintained rewrite): a new base
image under the run's version's atom and relation ids, holding only the parts the form states.
- **Its rules**: the form's, written with the builder from the stratum's rules read by their handles.
- **Its parts**: the declarations, facts, inputs, outputs and about rows its stratum mentions, as the one-stratum
  program stated them: nothing of the run's that the form does not name.
- **Its `Maint`** (decision 0215): its strata derived again, an overlay of the derivation, and its sets run
  together.

So its cost is the form's size, and its fields equal the tree form's exactly. Its relations and atoms keep the run's
ids where the run has them, so a name the run and the form share is one id in both.

## Why not an overlay of the run's version

An overlay answers with its parent's rules and parts as well as its own. The run's version holds the stratum's own
rules beside the form's rules for it, and every declaration, fact, input, output, native, gate and qualifier of the
run. Every reader of the form (its strata, its heads and readers, its fields) would see them. Stating the form's
parts in the overlay means removing the run's, which costs the run's size, not the form's. It also folds: the
builder's size counts removals, and an overlay of an empty view folds at once. Each form would then copy every
declaration and fact of the run.

## Why not a view whose parts are cut

0173 §7 has one-stratum forms as views: "ranges of its rules, and the relations they need". The builder's views keep
their parent's parts, so a view that cuts its parent's relations too would be a new kind of version. t36 item 7 step 9
needs such a view for the split's parts (E1), with a fold that does not count a view's removals. That is a separate
need, with its own measures. A form adds every rule it holds, so it is not a view in any case: it would be an overlay
on one.

## Consequences

- The run adds a form as the version it is given (no lookup by the tree's address, no image made from a tree per
  form), and its again rules and deltas are its `Maint`'s.
- `maintain::rewrite_with` writes the form with the builder. The session's maintained part (E3) still passes a tree
  until t36 item 7 step 9; its caller decodes the derivation.
