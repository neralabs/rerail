# 0005: Named derived facts

Status: proposed
Date: 2026-10-25
Amends: 0004 (the transducer is the IR for local structure, not for every single-file condition)

## Question
Where do intermediate relations that several rules need, and relational per-document analyses (name-keyed joins, fixpoints over names, data flow), live? Affects all three targets: per-edit (incremental facts), full repository in 1–2 s (compute once, share), outer SQL query (facts are SQL relations).

## Prior work consulted
- PR 2539 `docs/design/DECLARATIVE_LINTER_SQL_ONLY_BOUNDARY.md@281f500d7c` :8-29 (framework-owned generic facts only), :67-71 (operators including named queries), :105-106 (a relation named after a rule is suspect), :136-143 (dependency-cone invalidation), :169-186 ("Process Failure": Python prepares convenient rows, YAML queries them).
- PR 2539 `LINTER_GENERIC_FRAMEWORK_FACTS_DESIGN.md@281f500d7c` :93-147 (Pass 2 "Generic Derived Facts" belongs to fact materialization); `default_catalog.py` :54-141 (declared dependency maps); `project_fact_cache.py` :13-54.
- Moti facts-engine `docs/facts/DECLARED-FACTS.md` :53-54 (digest covers the lowered algebra, not its text); `facts_memo.rs` (content-hash memo keys).
- DLRE `c4bd76bf` shared syntax facts (−74.7% cold CPU); `shared-producer-workplan.md:271` (naming alone does not improve performance).
- lessons-ledger.md L11 (narrow per-family tables), L12 (interned atoms), L14 (per-file ownership), L16 (one physical owner per relation).
Agreement: agrees with PR 2539's design and implements what its tip left unbuilt (persistent per-fact cache, wired dependency cones).

## Decision
1. Every relation a rule may name is a fact with one producer: a **fact program** (rule-independent transducer program), a **native producer** (reviewed engine code), or a **derived fact** (declarative relational algebra compiled to SQL).
2. Facts are named and defined in `rerail.facts/v1` documents separate from rules (`engine/facts/catalog/`, `packs/<pack>/facts/`). No SQL text; what the algebra cannot state is a native producer.
3. Admission: concept-named (never rule-named), used by ≥ 2 rules or a catalog family, rows per document ≤ node count, no rule-specific seeds.
4. Identity by `fact_digest` over the lowered definition, schema and input digests; cached per (digest, tape identity) or per (digest, input generations); invalidated by dependency cone.
5. Transducer programs read facts as `SIDE` columns or `FACT` sets; SQL joins them with transducer outputs.
6. Keyed sets (E15) and bottom-up folds (E17) are withdrawn from the IR; lowering rule L5 and derived facts replace them.

Normative text: `spec.md` §21; `spec-transducer.md` §0, §2.11, §6.5.

## Evidence
`research/derived-facts.md`: inventory of 17 base facts and 8 derived facts the 60 data-layer rules share; derived facts total under 0.7M rows on the gate corpus. Measured component costs from `research/heavy-rule-profiles.md`: import resolution 0.43–0.68 s, call resolution 0.73–1.08 s (24.8 s in the bad form), class closure 0.28 s, recursive taint 0.49 s. Pending: M10 (L5 vs keyed sets), M11 (cold materialization ≤ 2.5 s).

## Consequences
- Rules become thinner; analyses are reviewed once in the catalog.
- The fact schema needs versioning discipline (digest-based; name clash is a compile error).
- An extra boundary crossing per document on the per-edit path, gated by M8/M10.
- Revisit if M11 fails or if L5 loses more than 1.5× to keyed sets on M7's inputs.
