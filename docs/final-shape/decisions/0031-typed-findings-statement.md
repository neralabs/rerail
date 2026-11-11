# 0031: The findings layer's statement is one typed query

Status: accepted (2026-11-03, list step 27)

## Context

Phase 8 row 3 says S's typed SQL shape (`findings::shape::sql_shape`) "becomes the lowering target for
the findings layer instead of a hand-built plan". Since decision 0013 the layer is RDL
(`waves::findings::FINDINGS`), planned by the planner. Since list step 20, each rule body is a typed
`logic::sql::select::Query`, checked before it is rendered. Three parts were still strings:

- **`emit::single`** made the one statement (decision 0013) from rendered text. It stripped the
  `INSERT OR IGNORE INTO new_<rel> ` prefix off each step, and split the views at `CREATE TEMP VIEW … AS`.
  It then joined the pieces into `WITH … AS MATERIALIZED (…)`, and nothing checked the whole statement.
  The policies' demand statement (`waves::policies`) goes through the same fold.
- **The batch inputs' sources** were subquery strings: four sites wrote
  `(SELECT a AS c0, … FROM t [WHERE batch = :batch])` by hand.
- **The layer's tables** were declared twice: in `FINDINGS`' `.decl`s, and again in a `TABLES`
  constant of column-list strings, which made both the DDL and the aliasing subqueries.

S's `sql_shape` can't express this program. It allows only equijoins (`Select::predicates` are
equalities) and integer columns. It has no anti-join, comparison or aggregate, while `FINDINGS` uses
negation (`!quiet`), ranges (`$s <= $b`) and `count`.

## Prior work consulted

- **Lessons ledger:**
  - **#6** (`AS MATERIALIZED` and naming don't bound evaluation; N1562 generated one clause's 164 MB as
    text). **Agrees:** the fold is now composed and checked as a structure, not concatenated as text.
    Each relation is still one fenced expression, as 0013 measured (41 ms on 48 fixtures). This step
    adds no new materialization.
  - **#7** (CROSS JOIN fences, plus SSC's emitted-plan audit test). **Agrees:** the planner's blocks
    keep their `CROSS JOIN` order inside the fold, and the whole statement is checked again.
  - **#16** (fix plans and candidate sets before fusing). **Agrees:** this step changes no plan. The
    blocks are the planner's, and only their reads of derived relations are rebound.
  - **#19** (fragments, not an interpreter). **Agrees:** the fold is relational composition, with one
    CTE per relation, not one per operator (N1562's 8,397 CTEs).
- **ast-index-spike** (`prior-work/ast-index-spike-design-index.md`):
  - **§5, fused plans beat materialized intermediates.** **Departs, as 0013 did:** the negated
    suppression strata are read more than once (`anchored`, `honoured`, `resolved`), so each is
    fenced once. The output's own expression is not materialized, because only the statement reads it.
  - **§6, planning is one statement over the whole batch.** **Agrees:** one statement per layer.
- **Decision 0013 (findings layer) and list step 20 (`logic::sql::select`):** the block form is
  kept, because the planner's Selinger order needs flat `CROSS JOIN` blocks. N's operator graph
  (`SqlAst`) is not used for the same reason.
- **S's `sql_shape`:**
  - **Adopted:** its source declarations (a table, its columns, identifiers checked), its fenced
    derived expressions (`shape_query … AS MATERIALIZED`), and checking the whole statement before
    emitting it.
  - **Not adopted as the target:** its query language (equijoins over integers). S's
    `generated_shape` is the hand-built plan the row meant, and the planner already replaces it.
  - The copy stays as copied, test-only, for O19.

## Decision

1. **Steps keep their checked queries** (`emit::Emitted::queries`, `views`, `inputs`). The planner's
   `checked_query` is what `sql_select` renders.
2. **`single` composes them into one `select::Query`:**
   - each view, then each relation below the output in stratum order, is a CTE;
   - a relation's steps are the arms of a distinct union, and the CTE is `MATERIALIZED`;
   - the output is the last CTE, not materialized, and the body selects it;
   - each block's reads of derived relations are rebound to their CTEs (`Select::relations_mut`);
   - the whole query is checked: it reads only the program's inputs, it reads no derived table, and
     each CTE is read only after it is defined.

   It refuses:
   - reading a delta, a walk's CTE, or constant rows;
   - a relation read in its own stratum or a later one;
   - a lattice.
3. **Batch inputs are typed sources** (`select::Source`): a table and its columns, read
   positionally (`c{i}`) or under their own names, and optionally filtered to the chunk's batch. The
   check validates each source's identifiers. `render` reproduces the earlier strings exactly, so
   compiled-plan identities (`waves::matches`) do not change.
4. **The layer's tables come from the program:** `Layer::tables` are `FINDINGS`' inputs other than
   the wave's two (`forbidden` from `violation`, `span`), with their declared columns. The DDL, the
   inserts and the sources all come from them, and `TABLES` is gone.

## Consequences

- One query type, `select::Query`, describes every statement the planner emits, including the
  folded findings and demand statements.
- The rendered statement differs from 0013's only in form:
  - `WITH RECURSIVE` with explicit `(c0, …)` columns;
  - a materialized CTE per view, as before;
  - the output's arms as a CTE the body reads, instead of `SELECT * FROM (…) UNION …`.
- S's `sql_shape` and `generated_shape` stay test-only, for O19.
