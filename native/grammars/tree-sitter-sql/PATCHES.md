# tree-sitter-sql: vendored with patches

Upstream: https://github.com/DerekStride/tree-sitter-sql, version 0.3.11 (commit
`7b51ecda191d36b92f5a90a8d1bc3faef1c7b8b8`), MIT. Rerail reads every `.sql` file with this one grammar
(`languages/sql.json`): PostgreSQL, SQLite, MySQL and ANSI files alike. The patches make it read what the
SQL pack's oracles and corpora hold: SQLFluff 4.4.0's rule fixtures (`packs/sql/upstream/sqlfluff/fixtures/`),
PostgreSQL 18.6's names, values and arguments oracles (`fixtures/sql/{scope,matcher/arguments}/postgres.sql`)
and the port-readiness corpora (`.rerail/port-readiness/corpus/rerail/`), following PostgreSQL's `gram.y`
wherever the two dialects differ. The oracle inputs and `fixtures/sql/suppress/sqlfluff/postgres_bodies.sql`
parse without an `ERROR` node. The 277 corpus files the grammar census reads (202 postgres, 71 mattermost,
4 anki) all parse without an `ERROR` or `MISSING` node. Of the 283
fixtures `packs/sql/README.md` scores, 265 from SQLFluff and 18 Rerail wrote (`fixtures/rerail/`), 274 do. The
9 that do not are all SQLFluff's:

- a bare `[1]` array literal (BigQuery syntax): `ansi/ST10/test_pass_array_access.sql`;
- a redundantly bracketed join target: `ansi/RF01/test_{fail_redundantly_bracketed_join_target_unknown_table,pass_redundantly_bracketed_join_target_alias}.sql`;
- `CREATE TRIGGER … BEGIN … END`: the three `sqlite/RF01/test_sqlite_create_trigger_after_*_reference.sql`;
- quoting the grammar reads differently: `ansi/AL05/test_pass_quoted_case_insensitive.sql` and
  `sqlite/AL05/test_pass_sqlite_different_escaped_quotes.sql`;
- a select list ending in a comma, which PostgreSQL rejects too: `ansi/AL08/test_fail_locs.sql`.

A port that skips files with syntax errors skips these (`packs/sql/README.md`); each finding they affect is a
declared divergence or scored as a syntax-error file.

## Files

| File | Origin |
| --- | --- |
| `LICENSE` | upstream, verbatim |
| `src/grammar.json` | generated from the patched `grammar.js` |
| `src/scanner.c` | upstream, with patches 11, 22, 23, 24, 25 and 29 |
| `src/tree_sitter/{alloc,array,parser}.h` | written by `tree-sitter generate` 0.27.0 |
| `rerail.patch` | the patches: `git diff` of `grammar.js`, `src/scanner.c`, `test/corpus/` and `queries/highlights.scm` |

Upstream ships no `src/grammar.json` or `src/parser.c`; `native/build.rs` generates the parser from
`grammar.json` with tree-sitter-generate 0.27.0, as it does the other vendored grammars.

## Regenerating

```sh
git clone https://github.com/DerekStride/tree-sitter-sql
cd tree-sitter-sql
git checkout 7b51ecda191d36b92f5a90a8d1bc3faef1c7b8b8
git apply <rerail>/native/grammars/tree-sitter-sql/rerail.patch
tree-sitter generate   # tree-sitter-cli 0.27.0
tree-sitter test       # 502 of 502 (1 case skipped, below)
```

then copy `src/grammar.json`, `src/scanner.c` and `src/tree_sitter/*.h` here.

The parse table holds 49,835 actions (tree-sitter caps it at 65,535) in 31,027 states. An external token in a
state's lookahead keeps tree-sitter from merging that state with any state that lacks it, so the grammar keeps
external tokens out of the states that end a statement (patch 25): with one there, every expression and type
state existed twice and the table did not fit.

The grammar has 985 node kinds (982 symbols and 3 aliases) and 61 field names. Rerail's tapes code a node kind
in ten bits and a field in six (`native/src/code_rule_dsl/tape.rs`: at most 1024 kinds and 63 fields), so
patch 103 keeps the count under that: new statement kinds share a node, and hidden rules used in few places
are inlined (`inline`), which costs no node kind but grows the parse table.

## Patches

Each patch has a case in `test/corpus/rerail.txt`, a file `rerail.patch` adds; `rerail.patch` holds
them all. Numbers 30–39 and 76–79 are unused. Each `grammar.js` change carries a `// Rerail patch N:` comment
naming the `gram.y` rule it follows.

1. **One comment kind.** Block comments were `marginalia`; they are `comment`, as `--` comments are, so
   the engine's comment handling (`kind: "comment"`, the SQLFluff `noqa` index) reads both. Upstream's
   five marginalia cases now expect `(comment)`.
2. **A join without `ON` or `USING`** (`FROM foo JOIN bar`), which SQLFluff's ANSI dialect reads:
   `ansi/AM05/*_default.sql`, `ansi/AL04/test_fail_subquery_join_matches_base_table.sql`.
3. **`UNION | INTERSECT | EXCEPT DISTINCT`**: `ansi/AM02/*`.
4. **`REINDEX` and `REFRESH MATERIALIZED VIEW [CONCURRENTLY]`** (`reindex_statement`,
   `refresh_statement`): `postgres/PG01/*reindex*`, `*refresh*`.
5. **`NOT VALID`** after an added constraint: `postgres/PG02/*not_valid*`.
6. **Doubled quotes inside quoted identifiers** (`"x""y"`, `` `p``q` ``):
   `mysql/AL09/test_pass_mysql_quoted_identifiers.sql`.
7. **`$` inside an identifier** after its first character (PostgreSQL):
   `postgres/RF05/test_pass_ignore_lists_unquoted_fail.sql`.
8. **A string alias after `AS`** (`a AS 'alias'`, MySQL and SQLite):
   `ansi/RF05/test_pass_ignore_lists_quoted_fail.sql`. Without `AS` a string after a term stays a string
   continuation (`U&'..'\n'..'`).
9. **`EXCLUDE [USING method] (element WITH operator, ...)`** table constraints:
   `postgres/CV05/test_exclude_constraint.sql`.
10. **MySQL `SET a = 1, b = @@var`**: several assignments, `@@` system variables (`system_variable`):
    `mysql/CV05/test_mysql_system_variable.sql`.
11. **Nested block comments**, as PostgreSQL's scanner and the SQL standard read them: `comment` is an
    external token scanned in `scanner.c` (`scan_comment`), so `/* a /* b */ c */` is one comment
    (`fixtures/sql/suppress/sqlfluff/postgres_bodies.sql` line 8).
12. **Named arguments** `f(x => 1)`, `f(x := 1)` (`named_argument`, fields `name` and `value`); upstream
    read `x =` then `> 1`.
13. **A double-quoted token in an expression is an identifier** (ANSI SQL, PostgreSQL, SQLite), never a
    string: `literal` takes `_expression_string` (single-quoted, dollar-quoted and `E''` strings);
    statement slots that take a string (`COMMENT ... IS`, `SEPARATOR`, options) keep accepting `"..."`.
    Upstream cases that read MySQL's double-quoted strings in expressions now expect `field`
    (`Mixed Predicate types`, `Disjunctive Predicate`, `GROUP CONCAT Impala`, the INSERT cases,
    `JSON traversal`, `Simple select with where and string`, `SELECT AS UNSIGNED`).
14. **Exponents in either case** (`4664.E+5`, `1E3`).
15. **`ON CONFLICT (target) [WHERE ...]` and `ON CONFLICT ON CONSTRAINT name`** (PostgreSQL).
16. **`DELETE FROM t [alias] USING from_item, ...`** (PostgreSQL).
17. (none: a `LATERAL` subquery in a comma-separated `FROM` list still fails; upstream reads only
    `CROSS JOIN LATERAL` and `JOIN LATERAL`.)
18. **`E''` strings with any backslash escape and `''`, and `UESCAPE`** after `U&'..'` and `U&".."`.
19. **PostgreSQL's SQL-syntax calls** `substring(s FROM a FOR b)`, `substring(s FOR b)`,
    `overlay(s PLACING t FROM a FOR b)`, `position(a IN s)`, `trim(BOTH | LEADING | TRAILING [x] FROM s)`:
    their operands are `parameter`s. MySQL's `EXTRACT(unit FROM x)` form keeps priority.
20. **`ALTER TABLE ... VALIDATE CONSTRAINT name`** (`validate_constraint`):
    `postgres/PG02/test_pass_add_foreign_key_not_valid_then_validate.sql`.
21. **Any non-ASCII character in an identifier**, as PostgreSQL's scanner reads them (`scan.l`'s
    `ident_start` and `ident_cont` take every byte from `\200` to `\377`, so every UTF-8 encoded
    non-ASCII character); upstream took `U+00C0` to `U+017F` only. A code rule's pattern metavariable
    (`$A`, read as `µA`, the expando ast-grep and Rerail's patterns share) is then an identifier.
22. **Dollar-quoted strings close at their own tag** (`scanner.c`). Upstream stored a tag's code points
    as `char`, so `$š$` (U+0161) read as `$a$` and closed at an embedded `$a$`; and it scanned each
    `$...$` candidate whole, so a candidate that failed consumed the `$` opening the real closing tag
    (`$a$foo$bar$a$`, the string `foo$bar`, never closed). Tags are now whole code points, at most 255
    of them (a longer one is not read as a tag, so the start tag a function body leaves open always
    serializes), compared by length and content; the closing tag is matched one code point at a time,
    a failed candidate leaving a `$` it stopped at to start the next. The scanner state is a fixed
    struct, serialized as a flag byte and the tag's code points.
23. **A dollar-quote tag is empty or an identifier** (`scanner.c`, `scan_dollar_string_tag`), as
    PostgreSQL's scanner reads it (`scan.l`'s `dolqdelim`: `[A-Za-z_]` or any non-ASCII code point,
    then those or digits). Upstream took any code points but `$` and white space, so `VALUES ($1,$2)`
    opened a string with the tag `1,` that ran to the next `$1,$` in the file, and each such tag with no
    close scanned on to the end of the file, so many parameters made a file quadratic. A candidate that
    is no tag now fails at its first other code point, before any body is read; `$1` stays a
    `parameter`.
24. **Routine bodies as string literals; DO; CREATE PROCEDURE; argument modes** (CreateFunctionStmt, DoStmt,
    func_arg). `AS $$ ... $$`, `AS '...'` and a C function's `AS 'obj_file', 'link_symbol'` are `literal`s in
    `function_body`, whatever the language, as PostgreSQL stores them and SQLFluff's postgres dialect lexes
    them; upstream parsed `$$` bodies as SQL, so a PL/pgSQL body (`IF`, `:=`, `RAISE`) failed. The scanner keeps
    no state: a dollar-quoted string is one token wherever it occurs (the start/end-tag tokens, `dollar_quote`
    and `function_declaration` are gone). New `do_statement` (`DO [LANGUAGE l] code [LANGUAGE l]`),
    `create_procedure`, the options (`function_transform`, `function_set`, `WINDOW`, `LANGUAGE 'name'`) in any
    order with the `AS` body among them, optional RETURNS, an argument mode before or after its name, and any
    expression as a default. A SQL-standard body comes last (patch 101).
25. **psql meta-commands**, as psql reads a script and SQLFluff's postgres dialect lexes it: `\echo ...`, `\if`,
    `\quit`, `\d+ t` are `meta_command` extras scanned in `scanner.c` (`scan_meta_command`), one token each,
    nothing inside is SQL; a command's arguments run to the end of the line, to a backslash that starts the
    next command, or through a `\\` separator; `\set` and `\copy` take the whole line, `\copy (query)` across
    lines. A command that sends the query buffer (`\g`, `\gx`, `\gset`, `\gexec`, `\gdesc`, `\crosstabview`,
    `\watch`) ends the statement before it, as `;` does, and alone re-sends the previous query; it is the
    internal regex token `_meta_command_send` (aliased `meta_command`), which the scanner leaves to the lexer:
    as an external token in the lookahead of every statement's last state it doubled the parse table.
26. **psql variables** `:name`, `:'name'`, `:"name"` are `parameter`s, as SQLFluff reads them (psql_variable).
27. **ALTER TABLE ... SET (reloption, ...) / RESET (...)** (`set_storage_parameters`,
    `reset_storage_parameters`; gram.y's reloption_elem, dotted names and identifier values), **ALTER COLUMN
    ... TYPE t [COLLATE c] [USING expr]**, and joins in **DELETE ... USING** (MySQL's multi-table DELETE too).
28. **DROP TABLE/VIEW/TYPE/SEQUENCE a, b**, **DROP MATERIALIZED VIEW** (`drop_materialized_view`), and
    **DROP FUNCTION | PROCEDURE name(argtypes), ...** (`drop_procedure`).
29. **Inline COPY data** after `COPY ... FROM STDIN;`: the rows psql sends, through the first line holding
    `\.` alone, are one external `copy_data` token (`scanner.c`, `scan_copy_data`), as pg_dump writes them and
    SQLFluff's postgres dialect lexes them (postgres_copy_stdin_data_block). Without such a line the COPY is a
    statement like any other.

Patches 40–59 read PostgreSQL's utility and privilege statements:

40. **GRANT and REVOKE** of privileges ON a target and of roles, and **ALTER DEFAULT PRIVILEGES** (GrantStmt,
    RevokeStmt, GrantRoleStmt, RevokeRoleStmt, AlterDefaultPrivilegesStmt; function_with_argtypes).
41. **ALTER FUNCTION | PROCEDURE | ROUTINE** options, RESET, rename, owner, schema, `[NO] DEPENDS ON
    EXTENSION`, and **ALTER AGGREGATE** (AlterFunctionStmt, FunctionSetResetClause).
42. **CREATE EXTENSION options in any order** and **ALTER EXTENSION** UPDATE / ADD / DROP object / SET SCHEMA
    (create_extension_opt_list, AlterExtensionStmt, AlterExtensionContentsStmt).
43. **SECURITY LABEL** and the other **COMMENT ON** targets: access methods, aggregates, operators
    (`op (left, right)`, any_operator), operator classes and families, text search objects, transforms,
    constraints, policies and rules on their table, large objects (CommentStmt, SecLabelStmt).
44. **CREATE CAST and DROP CAST** (CreateCastStmt, `(source AS target)`).
45. **Definitions**: CREATE AGGREGATE (aggr_args, and the pre-8.2 form), OPERATOR, COLLATION, TEXT SEARCH
    PARSER | DICTIONARY | TEMPLATE | CONFIGURATION and TYPE with gram.y's definition, whose def_arg is a type,
    an operator, a number, a string or NONE; DROP AGGREGATE and DROP OPERATOR (RemoveAggrStmt, RemoveOperStmt).
46. **CREATE, ALTER and DROP DOMAIN** (CreateDomainStmt's ColQualList, AlterDomainStmt). A DEFAULT is gram.y's
    b_expr, so it ends before COLLATE and AT, which start the next qualifier (`DEFAULT 1 COLLATE "C"`).
47. **Foreign data**: CREATE / ALTER / DROP FOREIGN DATA WRAPPER, SERVER, USER MAPPING and FOREIGN TABLE,
    IMPORT FOREIGN SCHEMA, with gram.y's create_generic_options and alter_generic_options.
48. **DROP TRIGGER | RULE | POLICY name ON table** (object_type_name_on_any_name) and **CREATE RULE**
    (RuleStmt: `DO [ALSO | INSTEAD] NOTHING | command | (command; ...)`).
49. **DISCARD, NOTIFY, LISTEN, UNLISTEN, CHECKPOINT and LOAD**.
50. **CALL procedure(arguments)** (CallStmt).
51. **PREPARE name [(types)] AS statement, EXECUTE and DEALLOCATE** (PrepareStmt, ExecuteStmt,
    DeallocateStmt); the prepared statement is a query or a DML statement, MERGE among them.
52. **VACUUM, ANALYZE, CLUSTER and REINDEX** with gram.y's utility_option_list `(name [value], ...)`, the
    legacy option words and vacuum_relation lists.
53. **SET value lists** (generic_set: `SET search_path = a, b`), `SET var FROM CURRENT`, `SET [LOCAL]
    TRANSACTION` and transaction_mode_list in any order, with or without commas.
54. **Transaction control**: BEGIN | START TRANSACTION [modes] ... COMMIT | ROLLBACK [AND [NO] CHAIN], as
    gram.y's TransactionStmt and TransactionStmtLegacy read both ends; a `BEGIN;` whose end the file does not
    hold is one statement, and SAVEPOINT, RELEASE, PREPARE TRANSACTION and the others stand alone.
55. **DROP OWNED, REASSIGN OWNED** and DROP of the other object kinds of DropStmt, RemoveFuncStmt (ROUTINE),
    DropOpClassStmt, DropOpFamilyStmt, DropTransformStmt and DropTableSpaceStmt.
56. **Role options and ALTER ROLE | DATABASE ... SET** with generic_set's dotted names and value lists.
57. **LOCK [TABLE] [ONLY] name [*], ... [IN mode MODE] [NOWAIT]** (LockStmt).
58. **CREATE of other kinds**: PUBLICATION, SUBSCRIPTION, STATISTICS, EVENT TRIGGER, ACCESS METHOD, LANGUAGE,
    TRANSFORM, CONVERSION, TABLESPACE, OPERATOR CLASS and FAMILY, POLICY, and createdb_opt_item's DATABASE
    options (`CONNECTION LIMIT`, `name [=] value | DEFAULT`).
59. **ALTER of other kinds**: PUBLICATION, SUBSCRIPTION, OPERATOR, OPERATOR CLASS and FAMILY, POLICY, TRIGGER,
    RULE, MATERIALIZED VIEW, COLLATION, CONVERSION, STATISTICS, TEXT SEARCH, LANGUAGE, EVENT TRIGGER,
    TABLESPACE, LARGE OBJECT, SYSTEM, VIEW column defaults and options; schema-qualified index and type names,
    def_arg reloption values (`= off`) and `[NO] DEPENDS ON EXTENSION`.

Patches 60–75 read PostgreSQL's tables, indexes and DML statements:

60. **CREATE TABLE as CreateStmt reads it**: OptTemp (TEMP, LOCAL/GLOBAL TEMP, UNLOGGED), an element list of
    columns, table constraints and LIKE clauses in any order or none (`()`), INHERITS, PARTITION BY (column,
    call or expression, each with COLLATE and an operator class), USING method, WITH (reloptions), ON COMMIT,
    TABLESPACE; KEY and INDEX name columns, as they are unreserved in PostgreSQL. Table constraints are
    siblings of the column definitions (the `constraints` wrapper is gone).
61. **CREATE TABLE ... PARTITION OF parent** with PartitionBoundSpec (`FOR VALUES IN | FROM ... TO | WITH
    (MODULUS, REMAINDER)`, `DEFAULT`).
62. **CREATE TABLE ... OF type** (OptTypedTableElementList: `name WITH OPTIONS constraints`).
63. **Column constraints** as ColConstraint reads them: `[CONSTRAINT name]` before any of them, GENERATED
    ALWAYS | BY DEFAULT AS IDENTITY [(sequence options)], GENERATED ALWAYS AS (expr) STORED, REFERENCES with
    key_match and key_actions, COLLATE, STORAGE, COMPRESSION, DEFERRABLE / INITIALLY attributes, and a DEFAULT
    that is gram.y's b_expr: it ends before the NOT that starts `NOT NULL`, and before COLLATE and AT.
64. **Table constraints** (TableConstraint, ConstraintElem, ConstraintAttributeElem) and EXCLUDE's
    opt_c_include, opt_definition, OptConsTableSpace and WHERE; ALTER TABLE ... ADD constraint without a name.
65. **INSERT DEFAULT VALUES, OVERRIDING {USER | SYSTEM} VALUE**, and conflict targets of index_elem
    expressions with COLLATE and an operator class.
66. **TRUNCATE [TABLE] relation_expr, ... [RESTART | CONTINUE IDENTITY] [CASCADE | RESTRICT]** (TruncateStmt;
    relation_expr: `name`, `name *`, `ONLY name`, `ONLY (name)`).
67. **COPY** as CopyStmt reads it: BINARY, column lists, `COPY (query) TO`, PROGRAM, STDIN / STDOUT, a psql
    variable as the file, `[USING] DELIMITERS`, the generic options and the pre-9.0 option words, WHERE. A
    generic option's string value (`DELIMITER ';'`, `ENCODING 'UTF8'`) is a `literal`, the string constant
    gram.y's copy_generic_opt_arg reads, not an `identifier`.
68. **CREATE INDEX** options (IndexStmt): any access method, ON ONLY / `*`, INCLUDE, NULLS [NOT] DISTINCT,
    WITH (reloptions), TABLESPACE, WHERE.
69. **ALTER TABLE actions** (AlterTableStmt): IF EXISTS, ONLY / `*`, ALL IN TABLESPACE, ATTACH / DETACH
    PARTITION, the other alter_table_cmd forms and the column forms (SET EXPRESSION, DROP EXPRESSION, identity
    options, SET STATISTICS, RESET (options), DROP ... RESTRICT | CASCADE).
70. **UPDATE and DELETE** (UpdateStmt, DeleteStmt): `t *`, `(a, b) = (row | ROW(...) | subquery)` set targets,
    subscripted targets (`a[1] = 2`), WHERE CURRENT OF cursor.
71. **CREATE VIEW and MATERIALIZED VIEW options** (ViewStmt's OptTemp and reloptions; CreateMatViewStmt's
    UNLOGGED, column list, access method, reloptions and tablespace).
72. **Sequence options** (SeqOptElem, SeqOptList) shared by CREATE / ALTER SEQUENCE and identity columns, the
    numbers NumericOnly.
73. **MERGE** (MergeStmt) with a WITH clause, `WHEN MATCHED | NOT MATCHED BY SOURCE | NOT MATCHED [BY TARGET]
    [AND cond] THEN UPDATE | DELETE | INSERT | INSERT DEFAULT VALUES | DO NOTHING`, and RETURNING.
74. **ALTER INDEX ... ATTACH PARTITION index** (index_partition_cmd).
75. **CREATE TABLE AS** column lists, WITH [NO] DATA and `AS EXECUTE name (params)`; **SELECT INTO**
    {[LOCAL | GLOBAL] TEMP | UNLOGGED} [TABLE] name (into_clause).

Patches 80–101 read PostgreSQL's queries, expressions and routine signatures:

80. **Unary minus and plus on any operand; TYPECAST binds tightest**, then `[`, then UMINUS, as gram.y's
    precedences do; a sign directly before a number stays in the numeric literal (doNegate), and a type
    keeps its array bounds (`x::int[][]`).
81. **Any PostgreSQL operator** as scan.l's `{operator}` reads it (`=-1` is `=` then `-1`), and
    `OPERATOR(schema.op)` as a binary or prefix operator (qual_Op).
82. **`a COLLATE name`** (`collate_expression`, %left COLLATE).
83. **`[NOT] LIKE | ILIKE | SIMILAR TO ... ESCAPE ...`**, ESCAPE binding tighter than LIKE.
84. **Postfix predicates**: ISNULL, NOTNULL, IS [NOT] [form] NORMALIZED, IS [NOT] UNKNOWN | DOCUMENT, IS [NOT]
    JSON [VALUE | ARRAY | OBJECT | SCALAR] [WITH | WITHOUT UNIQUE [KEYS]].
85. **AT TIME ZONE and AT LOCAL** (%left AT, binding tighter than `^`).
86. **from_list**: table_refs separated by commas, any of them a joined table (`FROM a JOIN b ON x, c`),
    LATERAL subqueries in any table_ref, and `USING (cols) AS alias` (join_qual).
87. **func_table and func_alias_clause**: functions in FROM with WITH ORDINALITY, ROWS FROM (...), and column
    definition lists (`AS t(a int)`); `rows` stays a table name.
88. **Parenthesized joined tables** (`'(' joined_table ')' [alias]`), nesting.
89. **Select clauses without FROM** (WHERE, GROUP BY, HAVING, WINDOW, ORDER BY, LIMIT), HAVING without
    GROUP BY.
90. **VALUES** as a statement, a CTE body, a subquery and a set operand, with ORDER BY and LIMIT (values_clause
    is a simple_select); INSERT's rows stay inline.
91. **Typed literals** (`timestamp(2) without time zone 'x'`, `func_name 'x'`, ConstInterval with
    opt_interval fields), and TIME, TIMESTAMP and INTERVAL as column names (col_name_keyword).
92. **VARIADIC arguments** (`f(a, VARIADIC b ORDER BY c)`).
93. **NUMERIC / DECIMAL with a signed scale** (`numeric(3, -6)`).
94. **Nested array constructors** (`ARRAY[[1, 2], [3]]`, `ARRAY[]`).
95. **Field selection** `(x).f`, `(x).*` and parenthesized whole-row references `(t.*)`.
96. **simple_select's set quantifier**: ALL, DISTINCT ON (exprs); GROUP BY [ALL | DISTINCT] with grouping
    sets, ROLLUP, CUBE and `()`; ORDER BY ... USING op; WINDOW lists and windows built on a named window.
97. **select_no_parens' tail**: LIMIT ALL, OFFSET ... ROW(S), FETCH FIRST | NEXT ... ONLY | WITH TIES, FOR
    UPDATE | NO KEY UPDATE | SHARE | KEY SHARE [OF ...] [NOWAIT | SKIP LOCKED], ORDER BY and LIMIT of a whole
    set operation whose last operand is parenthesized, `TABLE name`, and a CTE's SEARCH and CYCLE clauses.
98. **ONLY and `*` per relation** in any table_ref, and **TABLESAMPLE method (args) [REPEATABLE (seed)]**.
99. **Column references with any number of parts** (columnref: ColId indirection): parts past a three-part
    `object_reference` follow it in the `field` (`a.b.c.d.e`), the reading with the fewest such parts winning.
100. **Argument names that are type keywords** (`regexp_match(string citext, ...)`, `OUT name text`): gram.y's
     param_name is a type_function_name, so the type names this grammar reads as keywords and PostgreSQL
     neither reserves nor treats as column-name keywords (`string`, `text`, `name`, `uuid`, `date`, `oid`,
     `regclass`, ...) name arguments, as `identifier`s.
101. **A SQL-standard routine body comes last and takes no `;`** (opt_createfunc_opt_list opt_routine_body):
     `RETURN a_expr` ends before the `;` that ends the CREATE statement, so two such functions in a row parse;
     the options, the `AS` body among them, come in any order before it.
102. **`substring(s FOR n FROM m)`**: gram.y's substr_list takes FOR before FROM too; the operands stay in
     source order (`matcher/arguments_sql.rs` passes them as PostgreSQL does, `(s, m, n)`).
103. **Node kinds under the tapes' limit** (1024), with no tree change but these:
     - one `drop_object` node for the DROP statements patches 44–55 added (cast, domain, trigger, rule, policy,
       aggregate, operator, operator class and family, foreign table, foreign data wrapper, server, user mapping,
       owned, collation, conversion, statistics, text search objects, access method, event trigger, language,
       publication, subscription, routine, transform, tablespace); the upstream `drop_*` nodes and
       `drop_materialized_view`, `drop_procedure` stay;
     - one `alter_object` node for the ALTER statements of aggregate, operator, operator class and family,
       policy, trigger, rule, collation, conversion, statistics, text search objects, language, event trigger,
       tablespace, large object, publication, subscription, foreign data wrapper, server and user mapping;
       `alter_function`, `alter_extension`, `alter_domain`, `alter_default_privileges`, `alter_foreign_table`,
       `alter_materialized_view` and `alter_system` stay;
     - one `create_object` node for CREATE AGGREGATE, OPERATOR, COLLATION, TEXT SEARCH ..., FOREIGN DATA
       WRAPPER, SERVER, USER MAPPING, PUBLICATION, SUBSCRIPTION, STATISTICS, EVENT TRIGGER, ACCESS METHOD,
       LANGUAGE, TRANSFORM, CONVERSION, TABLESPACE and OPERATOR FAMILY; `create_cast`, `create_domain`,
       `create_rule`, `create_foreign_table`, `create_operator_class` and `create_policy` stay;
     - VACUUM's option names that are no keyword elsewhere (`SKIP_LOCKED`, `INDEX_CLEANUP`, ...) and the
       privileges `CONNECT`, `USAGE`, `MAINTAIN` are `identifier`s, as gram.y's utility_option_name
       (NonReservedWord) and privilege (ColId) read them;
     - a keyword with several spellings (`int`/`integer`/`int4`, `like`/`ilike`, the serial and integer
       types) is one token, its node unchanged;
     - 78 hidden rules are inlined.

`functions.txt`'s `Function body with non alphabetic dollar quoted tags` is marked `:skip`: it reads
`$.$text$.$` as a string, which PostgreSQL rejects (patch 23). `queries/highlights.scm` drops `marginalia`
(patch 1) and `keyword_declare` (patch 24), node types the patched grammar no longer has, so `tree-sitter test`
runs.

Upstream cases whose expected trees changed (`test/corpus/`, besides `rerail.txt`):

- `comment.txt`: `Simple marginalia`, `Can parse delete with marginalia`, `Multiline marginalia, basic`,
  `Multiline marginalia, empty`, `Marginalia, more symbols` expect `(comment)` (patch 1).
- `casting.txt` `SELECT AS UNSIGNED`, `expressions.txt` `Mixed Predicate types` and `Disjunctive Predicate`,
  `functions.txt` `GROUP CONCAT Impala`, `insert.txt` `Insert with multple values`, `Insert with field name`,
  `Insert from unioned select` and `ON DUPLICATE KEY UPDATE`, `json.txt` `JSON traversal`, `select.txt`
  `Simple select with where and string`: a double-quoted token in an expression is a `field` (patch 13).
- `functions.txt`: `Function details, specified after string body`, `Dollar quoting a simple PLPGSQL body`,
  `Variable declarations`, `More complex function body`, `Function body with a $$ string literal` expect one
  `literal` body (patch 24); `errors.txt`'s `Function body with a $$ string literal with the same name` and
  `Function body with a unbalanced $$` still recover with an `ERROR` (PostgreSQL rejects them; the stray `END`
  after the first is a `transaction_statement`, patch 54), and `Function body with a tag name contains
  whitespace`, whose body is a valid string, parses whole (patches 23, 24).
- `create.txt`: `Create table with constraint`, `... constraint on multiple columns`, `... constraint checks`,
  `... auto increment`, `... multiple constraints`, `Create table with index`: table constraints are siblings
  of the column definitions, without a `constraints` wrapper (patch 60); `Create table with generated always`:
  `GENERATED ALWAYS AS IDENTITY` ends in `keyword_identity`, not a column named identity (patch 63).
- `alter.txt` `T-SQL: alter table add multiple columns at once`: `DEFAULT('EMPTY')` is a
  `parenthesized_expression`, not a `list` (patch 63's b_expr).
- `expressions.txt` `full text search operators`: `'rat'::tsquery` casts the literal alone, TYPECAST binding
  tighter than any operator (patch 80).
- `select.txt` `Simple select with ONLY`: `keyword_only` belongs to the `relation` (patch 98); `SELECT FROM
  UNNESTED ARRAY WITH ORDINALITY`: WITH ORDINALITY and the alias's column list belong to the function in FROM
  (patch 87).
- `insert.txt` `COPY FROM STDIN CSV WITH OPTIONS`: the string values of `DELIMITER`, `QUOTE`, `ESCAPE` and
  `NULL` are `literal`s (patch 67).
