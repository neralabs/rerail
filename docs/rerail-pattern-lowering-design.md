# Rerail pattern lowering design

This design now defines structural terminal lowering for the SQL rule engine.
The packed `PatternProgram` emits typed terminal rows.
SQL composes those rows with the complete clause plan.

The parser-independent tape model is in
[`generic-tape-v5-design.md`](generic-tape-v5-design.md).

## Scope

Do not port SCM support into Rerail.
Landing added SCM to exercise the executor with a large external query corpus.
SCM is not a Rerail input, intermediate representation, or production feature.

This design starts from Rerail's compiled ast-grep `Pattern`.
It lowers that value directly into a verified packed program.

Two independent compilation paths meet at execution time:

```text
source file
  -> language parser
  -> persisted TapeImage
  -> restored TapeDoc
  -> packed file tables

Rerail pattern source
  -> language pattern parser
  -> ast-grep PatternNode
  -> verified PatternProgram
  -> packed program tables

PatternProgram + packed file tables + candidate roots
  -> transducer backend
  -> matches and typed captures
```

A pattern does not lower into the tape.
The source file lowers into the tape.
The pattern lowers into a program that reads the tape.

## Blade syntax errors

### Why Blade has separate error ranges

Tree-sitter represents recovery in its tree.
It creates `ERROR` nodes and zero-width `MISSING` nodes.
Rerail can therefore preserve Tree-sitter syntax errors as ordinary taped nodes with flags.

Forte has a different parser contract.
Its successful parse reports lexer errors as byte offsets beside the tree.
It does not create syntax-error nodes for these offsets.
A complete Forte parse failure also has no useful partial tree.

Rerail converts a successful Forte error offset to a zero-width range:

```text
offset N -> N..N
```

The current code converts a complete Forte parse failure to the whole source range.
Forte parse failures are depth or work-limit failures, not proven source errors.
Tape version 5 must report them as evaluation failures instead.

The Blade adapter also builds one synthetic PHP program from embedded template expressions.
Tree-sitter parses that generated PHP program.
The adapter maps each PHP node back to the original Blade bytes.

The final Blade tape therefore contains two tree domains:

1. The Forte template tree.
2. The mapped embedded-PHP tree as the document root's last child.

Tree-sitter recovery nodes in the embedded PHP tree remain ordinary error or missing tape nodes.
Forte lexer and parse errors have no corresponding nodes.
Those errors require a side table.

### Why the ranges affect rule results

Rerail does not treat a recovered tree as complete evidence.
A parser can omit or invent structure near an error.
That difference is important for negative rules and counts.

`TapeDoc::syntax_error_ranges` combines these sources:

- Tree-sitter `ERROR` node spans.
- Tree-sitter `MISSING` node points.
- Forte lexer-error points.
- The whole Blade source after a complete Forte parse failure.

`SyntaxErrors` sorts and merges these ranges.
A zero-width missing point intersects a witness that touches that point.

The compiler classifies each matcher with `sound_under_errors`.
A local positive rule can run against clean regions.
A negative outward relation is not sound because an error can hide a related node.

The evaluator uses the classification in two ways:

- It rejects an unsound clause for a file that contains syntax errors.
- It drops a produced witness when the witness overlaps an error region.

Thus, the side table is not only diagnostic text.
It changes whether a policy result is valid.

### Why this data appears in a generic tape format

The original tape goal was parser-free restoration.
A restored `TapeDoc` had to reproduce all parser facts that affect matching.
Forte's external errors were one such fact.

The implementation added a generic field named `syntax_errors` to `TapeImage`.
Only the Blade path currently supplies entries.
Tree-sitter paths encode equivalent facts through node flags.

This decision was expedient, but the binary format extension mechanism is poor.
The format has twelve positional length fields without section identifiers.
Landing later reused the last position for SCM supertype paths.
Both branches retained `TMPTAPE4` and version 4.

The collision is evidence that the format is too positional.
The evaluator still needs error regions.
Version 5 derives them from generic `ERROR` and `MISSING` nodes instead of a Blade side table.

### Recommended format treatment

Keep Rerail tape version 4 during the first executor port.
Do not read Landing tape bytes.
Pack restored `TapeDoc` values into the new executor ABI.

Tape version 5 must encode one parser-independent tree over one source.
Tree-sitter and Forte adapters both produce the same generic node model.
Tree-sitter supertype paths become generic node classes.
Forte lexer errors become ordinary generic `ERROR` nodes.

The embedded PHP program becomes a separate generated-source document with its own generic tape.
A workspace source map maps PHP findings back to Blade bytes.
The tape contains no Blade mount, overlap, source-map, or diagnostic field.

The packed executor derives syntax-error ranges from generic `ERROR` and `MISSING` nodes.

## Current source tape

### Persisted rec6 representation

Rerail writes nodes in preorder.
Each primary node record is six bytes:

```text
u16 kind_and_field
u16 parent_distance
u16 subtree_size
```

`kind_and_field` uses these bits:

```text
bits 0..9   kind code
bits 10..15 field code
```

The field code is the node's field in its parent.
A zero field means no field.

`parent_distance` gives the backward distance to the parent.
`subtree_size` gives the number of records in the node's complete subtree.
A value of `u16::MAX` reads its real value from an overflow table.

Spans use a separate stream.
Each node stores a zigzag-varint start delta and a varint byte length.
Every sixteenth node has a checkpoint for bounded restoration work.

Sparse side tables record these facts:

- Extra nodes.
- Error nodes.
- Missing nodes.
- Contextual kind aliases.
- Kind overrides.
- Parent and subtree-size overflows.
- Version-4 unrepresented syntax-error ranges.

The image binds itself to one language, source length, and source digest.
Restoration rejects a different grammar or source revision.

### Restored representation

`TapeDoc` expands each node into navigation data:

```text
kind
field
start_byte
end_byte
subtree_size
parent
first_child
next_sibling
previous_sibling
child_count
extra flag
error flag
missing flag
```

The preorder layout gives descendants without a search:

```text
descendants(node) = node + 1 .. node + subtree_size
```

This representation already supports ast-grep matching.
The packed backend must consume the same logical facts.

### Proposed packed file representation

The packed ABI can use one fixed record for each node:

```text
PackedNodeV1 {
  u32 parent;
  u32 next_any;
  u32 next_named;
  u32 previous_any;
  u32 previous_named;
  u32 child_start;
  u32 child_count;
  u32 descendant_end;
  u32 source_start;
  u32 source_end;
  u32 kind_field_flags;
}
```

This record is 44 bytes.
`kind_field_flags` packs the small kind, field, and flag values.

The current Landing record also uses 44 bytes.
It lacks previous-sibling links and stores kind, field, and flags separately.
Rerail needs previous links for `follows` relations.
Packing the final values makes room without increasing the record size.

Direct child ordinals remain in a shared `u32` array.
Source bytes, line starts, and syntax-error ranges use per-file ranges into shared arrays.

```text
PackedFileV1 {
  node_start;
  node_count;
  child_start;
  child_count;
  source_start;
  source_length;
  line_start;
  line_count;
  syntax_error_start;
  syntax_error_count;
  grammar_id;
}
```

All packed node references are batch-global ordinals.
The batch builder converts local tape ordinals with `node_start + local_ordinal`.

## Rerail pattern compilation

### Pattern parsing

Rerail already performs the first lowering stage.
`rule_pattern` asks the language adapter to preprocess and parse pattern source.

For TypeScript, this example:

```text
const $NAME = $VALUE;
```

becomes an ast-grep `PatternNode` tree similar to this tree:

```text
Internal lexical_declaration
  Terminal "const" unnamed
  Internal variable_declarator
    MetaVar Capture(NAME, named=true)
    Terminal "=" unnamed
    MetaVar Capture(VALUE, named=true)
  Terminal ";" unnamed
```

The exact tree follows the pinned grammar.
The compiler must not reconstruct it from source text.

A contextual pattern first parses its complete context.
It then selects the requested internal node.
The selected node becomes the pattern root.
The root kind gate remains part of the compiled pattern.

All current production patterns use ast-grep's Smart strictness.
Rerail does not expose a policy key that selects another strictness.
The first packed format therefore needs exact Smart behavior only.

### Smart matching contract

Smart matching is not a normal tree equality operation.
The packed implementation must preserve these rules.

For an internal pattern node:

1. The candidate kind must equal the pattern kind.
2. Pattern children and candidate children are compared in order.
3. Extra candidate nodes, normally comments, can be skipped.
4. An unmatched anonymous candidate can be skipped.
5. A significant unmatched named candidate fails the pattern.
6. After all pattern children match, all remaining candidate children are accepted.

For a terminal pattern node:

- A named terminal requires equal kind and equal source text.
- An anonymous terminal requires equal kind.
- Anonymous terminal text is not compared.

For a nested metavariable:

- `$NAME` accepts one named node and retains it.
- `$$NAME` accepts one named or anonymous node and retains it.
- `$_` accepts one named node without retention.
- `$$_` accepts one named or anonymous node without retention.
- An extra candidate is skipped before nested metavariable matching.

A root metavariable is different.
Under Smart strictness, it can bind an extra root directly.
The executor must not apply the nested extra-node skip at the root.

### Capture equality

A repeated single capture does not mean equal node identity only.
Ast-grep uses this algorithm:

1. Equal node identity succeeds.
2. If either node is a named leaf, equal source text succeeds.
3. Otherwise, kinds and complete child structures must be equal.

A repeated multi-capture compares only named nodes from each captured sequence.
It compares each pair with the same exact-node algorithm.
Anonymous separators in the sequences do not affect equality.

The packed capture environment therefore needs typed values:

```text
CaptureValue
  Unbound
  Single(node)
  Multi(first_link, count)
  Text(byte_start, byte_length)
  Count(i64)
  Unknown
```

`CAPTURE` alone is insufficient.
The executor needs bind-or-compare operations.

### Ellipsis behavior

`$$$` and `$$$NAME` operate at the parent child-sequence level.
They do not match one node through normal metavariable logic.

Ast-grep uses the next nontrivial pattern node as the stopping probe.
It chooses the first candidate where that next node can match.
The probe uses a cloned capture environment.
Failed probes cannot leak bindings.

If the ellipsis ends the pattern, it consumes all remaining children.
If only anonymous pattern terminals follow it, those terminals determine how many trailing candidates the capture excludes.

Two adjacent ellipses consume at least one candidate between them.
This rule guarantees progress.

These details need a dedicated verified operation.
They must not become an ordinary greedy SCM repetition.

## Proposed `PatternProgram`

### Program tables

Extend the verified `Program` with a pattern subprogram table:

```text
ProgramV1
  headers
  rule_words
  pattern_words
  strings
  capture_slots
  kind_sets
  constants
  output_words
```

Each instruction remains one 32-bit word:

```text
bits 0..7   opcode
bits 8..15  mode
bits 16..31 operand
```

Wide operands use verified side-table entries.

The pattern instruction set needs these semantic operations:

```text
P_REQUIRE_KIND kind
P_MATCH_TERMINAL terminal_id
P_BIND_SINGLE capture_id, named_requirement
P_DROP_SINGLE named_requirement
P_ENTER_CHILDREN frame_id
P_NEXT_CHILD
P_END_CHILDREN_SMART
P_ELLIPSIS ellipsis_id
P_SAVE_NODE cursor_id
P_RESTORE_NODE cursor_id
P_JUMP target
P_FORK target
P_RETURN
P_ACCEPT
P_REJECT
```

`P_MATCH_TERMINAL` reads an immutable terminal record:

```text
PatternTerminal {
  kind;
  string;
  named;
}
```

`P_ELLIPSIS` reads this record:

```text
PatternEllipsis {
  capture_id_or_none;
  continuation_pc;
  trailing_anonymous_goal_count;
  adjacent_ellipsis;
}
```

The verifier computes the maximum node-stack depth from the pattern tree.
It rejects an invalid return, cursor slot, capture slot, or control-flow cycle.

The interpreter can initially execute these operations directly.
Metal and CPU JIT lowering then implement the same operations without host calls.

### Execution state

One pattern path needs this state:

```text
PatternPath {
  pc;
  root_node;
  current_node;
  node_stack;
  child_cursor_stack;
  capture_environment;
  multi_capture_builders;
  work_used;
}
```

The capture environment uses persistent links.
A fork shares old links and appends new links only on its own path.
This representation gives transactional branch behavior without copying a hash map.

A failed path discards its new links.
An accepted path publishes capture values in capture-slot order.

## Detailed lowering example

### Candidate source

Use this TypeScript source:

```typescript
const answer = compute(x);
```

A representative preorder tape is:

| Ordinal | Kind | Field | Parent | Subtree end | Span |
| ---: | --- | --- | ---: | ---: | --- |
| 0 | `program` | none | none | 13 | `0..26` |
| 1 | `lexical_declaration` | none | 0 | 13 | `0..26` |
| 2 | `const` | none | 1 | 3 | `0..5` |
| 3 | `variable_declarator` | none | 1 | 12 | `6..25` |
| 4 | `identifier` | `name` | 3 | 5 | `6..12` |
| 5 | `=` | none | 3 | 6 | `13..14` |
| 6 | `call_expression` | `value` | 3 | 12 | `15..25` |
| 7 | `identifier` | `function` | 6 | 8 | `15..22` |
| 8 | `arguments` | `arguments` | 6 | 12 | `22..25` |
| 9 | `(` | none | 8 | 10 | `22..23` |
| 10 | `identifier` | none | 8 | 11 | `23..24` |
| 11 | `)` | none | 8 | 12 | `24..25` |
| 12 | `;` | none | 1 | 13 | `25..26` |

Numeric kind and field values come from the pinned grammar.
The table uses names only for readability.

For ordinal 6, the persisted rec6 values are conceptually:

```text
kind_and_field = kind(call_expression) | field(value) << 10
parent_distance = 6 - 3 = 3
subtree_size = 6
span_delta_and_length = encoded separately
```

The packed batch changes local ordinals to batch-global ordinals.
It also adds direct navigation links and source-buffer offsets.

### Pattern source

The Rerail pattern is:

```text
const $NAME = $VALUE;
```

The anchor analysis finds `lexical_declaration` as the root kind.
The file anchor index supplies ordinal 1 as a candidate root.
The packed executor does not scan all 13 nodes for this pattern.

A readable lowering is:

```text
00 P_REQUIRE_KIND lexical_declaration
01 P_SAVE_NODE root
02 P_ENTER_CHILDREN frame_lexical
03 P_MATCH_TERMINAL terminal_const
04 P_NEXT_CHILD
05 P_REQUIRE_KIND variable_declarator
06 P_SAVE_NODE declarator
07 P_ENTER_CHILDREN frame_declarator
08 P_BIND_SINGLE NAME, named
09 P_NEXT_CHILD
10 P_MATCH_TERMINAL terminal_equals
11 P_NEXT_CHILD
12 P_BIND_SINGLE VALUE, named
13 P_END_CHILDREN_SMART
14 P_RESTORE_NODE declarator
15 P_RETURN
16 P_NEXT_CHILD
17 P_MATCH_TERMINAL terminal_semicolon
18 P_END_CHILDREN_SMART
19 P_RESTORE_NODE root
20 P_ACCEPT
```

The shown `P_RETURN` marks the end of the nested declarator subprogram.
An implementation can inline it and use verified continuation addresses.

Execution on the candidate does this work:

1. Ordinal 1 passes the root-kind test.
2. Ordinal 2 passes the anonymous `const` kind test.
3. Ordinal 3 passes the declarator kind test.
4. Ordinal 4 binds `NAME` to `6..12`.
5. Ordinal 5 passes the anonymous `=` kind test.
6. Ordinal 6 binds `VALUE` to `15..25`.
7. Ordinal 12 passes the anonymous semicolon kind test.
8. The program emits root ordinal 1 with two single captures.

The output before violation rendering is:

```text
PatternMatch {
  root: 1,
  captures: [
    Single { slot: NAME, node: 4 },
    Single { slot: VALUE, node: 6 },
  ]
}
```

Rerail's output program then derives the configured witness anchor, message, fix, and suppression result.

### Repeated capture example

For this pattern:

```text
$A + $A
```

The first `P_BIND_SINGLE A` stores its node.
The second operation becomes `P_BIND_OR_COMPARE_SINGLE A`.

These candidates behave differently:

```text
x + x       -> match
x + y       -> no match
foo() + foo() -> structural equality match
foo() + bar() -> no match
```

The executor must use ast-grep exact-node equality.
A source-span equality test or node-ordinal equality test is not sufficient.

### Multi-capture example

For this pattern:

```text
$F($$$ARGS)
```

The pattern parser produces a call node, one single capture, and one child-sequence ellipsis.
The arguments subprogram is conceptually:

```text
P_MATCH_TERMINAL "("
P_NEXT_CHILD
P_ELLIPSIS {
  capture: ARGS,
  continuation: end_of_arguments,
  trailing_anonymous_goal_count: 1
}
P_END_CHILDREN_SMART
```

The trailing anonymous count represents the closing `)` pattern node.
Ast-grep removes the corresponding final candidate from `ARGS`.
It does not lower this case as a generic greedy repetition.

For `f(a, b)`, the retained sequence can include separator nodes internally.
Repeated `$$$ARGS` equality compares only named nodes.
Text interpolation spans from the first retained node through the last retained node.

## Integration with complete Rerail rules

A `PatternProgram` answers only one `AstMatcher::Pattern` term.
A complete clause needs an outer verified rule program.

For this rule:

```json
{
  "pattern": "const $NAME = $VALUE;",
  "has": {
    "kind": "call_expression",
    "stopBy": "end"
  },
  "not": {
    "regex": "unsafe"
  }
}
```

The complete lowering is conceptually:

```text
RULE_BEGIN_TRANSACTION
CALL_PATTERN pattern_0
RELATION_BEGIN descendant, end
  TEST_KIND call_expression
RELATION_ACCEPT_FIRST
NOT_BEGIN_ISOLATED
  TEST_NODE_REGEX regex_0
NOT_END
RULE_COMMIT
OUTPUT violation_0
```

The rule program retains the compiler's term order.
It controls environment transactions, relations, unknown values, limits, and output behavior.
The pattern subprogram controls only ast-grep pattern semantics.

A clause enters the packed route only when every term and output operation lowers.
The executor never calls `GrammarMatcher` for an unsupported nested operation.

## Implementation sequence

1. Keep persisted Rerail tape version 4 unchanged.
2. Expose the required `TapeDoc` navigation methods to the packed module.
3. Define `PackedNodeV1`, `PackedFileV1`, and syntax-error ranges.
4. Add typed single and multi-capture values.
5. Add the verified `PatternProgram` instruction set.
6. Port Smart matching as a direct translation of ast-grep 0.45.3.
7. Add terminal, metavariable, repeated-capture, and ellipsis parity tests.
8. Lower `AstMatcher::Pattern` from its existing `PatternNode`.
9. Compare packed matches with `Pattern::match_node_with_env` on identical roots.
10. Add the outer rule and output programs after pattern parity is complete.
11. Add Metal lowering only after the Rust interpreter passes the pattern corpus.
12. Design tape version 5 separately from the executor port.

The current implementation stores a verified immutable `PackedPattern` tree in a packed `Program`.
The first `xFilter` runs this bounded producer against demanded roots.
The producer implements Smart terminals, metavariables, ellipses, rollback probes, and repeated-capture equality.
It emits typed single and multi-capture rows without a `GrammarMatcher` call.
Metal lowering remains incomplete, so the pattern producer currently runs on the CPU inside `xFilter`.

The first implementation must not support SCM.
It must not parse pattern source inside an executor.
It must not infer pattern structure from source strings after `PatternNode` exists.
