# Declared facts: the shared fact layer as a language construct

Status: **proposal under evaluation, not scheduled.** Written 2026-10-23 as the decider's reading of Jeremy's suggestion; no implementation decision has been taken. Origin: the measured residue of Phase 2 (34.9 s of byte-identical stage text recomputing the same relations per clause over clause-specific subsets; selection sharing admissible only under a subtree-closure side condition) and Jeremy's proposal that the language declare the facts it computes rather than the engine infer them. This document amends `INVARIANTS.md` I2 and is read under it.

## The idea

Today a rule carries its own copy of the relations it needs (`parameters`, `calls`, `ownership`, …) as SQL stages over its own matcher inputs. The engine recovers sharing by recognising structure (view substitution, CSE by digest, semijoin sharing under side conditions). That recovery is exact but partial, and it hides the fact layer inside rule text.

With declared facts, a pack states the relations it computes, once, as first-class definitions with declared structure. Rules read them by name. The engine computes each declared fact once per content (or once per membership), memoises it by digest, shares it across every rule and pack that names it, and exports it to the cross-file dataflow when it is membership-dependent. Nothing is inferred from rule text; the only inference left is type checking of what the author declared.

This is I2 ("facts are relations with declared structure; derived relations are views over the core, compiled like any rule") made visible in the language, and I7 (memoisation by expression digest) applied at the granularity the author chose.

## Surface

A `facts` block, at pack level or in a policy document, each entry a named relation:

```jsonc
"facts": {
  "py.parameters": {
    "over": "content",                       // declared: "content" | "membership"
    "columns": { "node": "int", "name": "text", "owner": "int", "ordinal": "int", "kind": "text" },
    "key": [["node"]],                        // declared keys
    "fd":  { "node": ["name", "owner", "ordinal", "kind"] },
    "define": {
      "sql": "SELECT p.id AS node, text(p.id) AS name, f.id AS owner, … FROM node f JOIN child c ON … WHERE f.kind = 'function_definition' …"
    },
    "version": 1
  },
  "py.import_identifiers": { "over": "content", "define": { "match": { "language": "python", "rule": { "kind": "import_from_statement", … } }, "project": { … } }, "key": [["node"]] },
  "proj.module_of":        { "over": "membership", "define": { "sql": "SELECT m.path, … FROM member m …" }, "key": [["path"]] }
}
```

- `define.sql`: SQL over the core relations (`node`, `child`, `field`, `identifier`, `line`, `string_value`, `py_binding`, …) and other declared facts; the subset is the compiler's (`IR.md`).
- `define.match`: a pattern-DSL matcher over all nodes of the content, with a projection; lowered by the pattern front end to the same algebra. This is the general form of what the dump inputs do today.
- `over`: the author's claim; the compiler infers the class from the definition's leaves (I6) and refuses a mismatch.
- `columns`, `key`, `fd`: declared structure (I2). Keys and FDs are checked: provable ones at compile time through the IR's metadata; the rest at runtime per content (a duplicate key is a named refusal of that content's fact, never a silent dedup).
- `version`: part of the digest; bump it when the definition changes meaning.

Rules reference facts by name wherever a stage or input table is allowed:

```jsonc
"stages": [
  { "name": "my_params", "query": "SELECT p.* FROM py.parameters p WHERE p.owner IN (SELECT node FROM selection)" }
]
```

The clause's own selection stays the clause's business; the shared relation is computed over all nodes, and the restriction is an ordinary semijoin. The subtree-closure side condition disappears because nothing is being rewritten; the author wrote the semijoin.

## Semantics

- A declared fact is a plan like any other: compiled once at pack load, `EXPLAIN`-able, audited, rewritten by the registry, exact or refused (I9).
- **Identity:** `(fact digest, schema version)` where the digest covers the definition's algebra after lowering, not its text; two packs declaring the same definition under different names share one computation and one memo entry.
- **Per-content facts** are memoised by `(content hash, fact digest, schema version)` in the content store, with completion words; a fact is computed once per content across the fleet, for every rule and worktree that reads it.
- **Membership facts** compile to the dataflow (delta plans generated, base + cluster + worktree tags) and are maintained incrementally; per-content facts they read are exported automatically (projection and distinct pushed down).
- **Reading across classes:** a per-content rule may read per-content facts only (I6, by construction: its `FileFacts` handle names them); a membership rule may read both.
- **Order:** a declared fact has no order unless it declares a total one (`order: [...]` proven total from keys), in which case consumers may rely on it; otherwise order-dependent consumers are `Total::Checked` as today.
- **Cost accounting:** the census and `EXPLAIN` report each fact's cost per file and its readers, so the fact layer's price is visible to the author instead of hidden in stage text.
- **Legacy text:** rules without `facts` keep working through the current inference (views, CSE, substitution rewrites). Inference is the compatibility path, not the design.

## What it changes in Phase 2's measured residue

- The 34.9 s of byte-identical stage text across batch-6 clauses becomes 12 declared facts computed once per content; the clauses' stages become semijoins over them. Expected: most of that 34.9 s, minus the semijoins (estimate; S9 measures).
- The dump inputs' shared selections (`quality-conditional-1-selection`, the state-tripwire gates) become declared per-content facts (`define.match`), computed once per content for every clause and rule that uses them, instead of once per rule per file.
- Exports to the dataflow stop being derived by the classifier from stage text; they are the membership facts' declared inputs.

## Open questions, for Jeremy

1. Naming and namespaces (`py.*` core-derived, `pack.*` pack-local, `proj.*` membership?), and who owns a shared fact's definition when two packs disagree.
2. Whether `define.match` facts may capture (project) spans and text, or only node ids (ids keep facts small and let consumers read text lazily from the source).
3. Versioning policy: digest only (automatic, invisible) or explicit `version` with a changelog obligation.
4. Whether the 11–12 shared relations should ship as a standard `py.*` library pack that rule authors read, with the data-layer rules migrated to it.
5. Budgets: a declared fact has no per-rule budget; what happens when one content's fact exceeds a work limit (named refusal of that content's fact for all readers is the current rule).
6. His compiler as `define.match`'s front end: it lowers a complete matcher with recursion and ordered capture; ours lowers a subset. Retargeting his lowering at the IR makes every `define.match` fact exact and complete.

## If adopted

- **A compiler slice would:** `facts` block in the compiler (`rerail-facts`): parse, type, digest, memo keys, reader binding in stages and inputs, census and `EXPLAIN` rows; migrate batch 6's shared stage texts into a pack-level `facts` block (definitions equal to the stage texts over the full node set, checked by S4's corpus differential); measure G2.3 and parity with the migrated pack; the original pack keeps working through inference.
- **With Jeremy:** the open questions above, then the `py.*` library pack and the migration of the 60 rules.
