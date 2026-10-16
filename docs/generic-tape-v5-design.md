# Generic tape version 5 design

## Correction

The previous syntax-view design kept multiple trees inside one tape.
That repeats the current architectural error with a more general name.

A generic tape must contain exactly these items:

- One source byte string.
- One grammar identity.
- One rooted syntax tree over that source.
- Generic node flags and optional abstract classes.

A tape must not contain another language's tree.
It must not contain source maps, embedded-language mounts, parser diagnostics, or backend-specific tables.

A source file can produce multiple independent analysis documents.
Each analysis document receives its own generic tape.
Mappings between derived documents and the physical file belong to the workspace, outside the tape format.

## Why Blade can use error nodes

Blade can map a Forte lexer-error offset to a node.
The current Blade tree builder already computes these facts:

- Every token's source range.
- Every token's owning flat node.
- Every final node's source range.
- Parent and sibling links.

The adapter can use the token owner when an error falls in a token.
It can use the deepest containing node for an offset between tokens.
It can use the current open construct or document root at end of file.

The resulting synthetic node is an ordinary generic `ERROR` node.
It has a zero-width span at the Forte error offset unless Forte supplies a reliable extent.
It enters the owning node's children in stable source order.

This insertion changes structural matching near the error.
That behavior is acceptable and consistent with Tree-sitter recovery nodes.
A recovered syntax tree must expose its recovery structure to tree matching.

The adapter must define and test the ownership rule.
The tape format does not know that the node came from Forte.

## Operational failures are not error nodes

The Forte port returns a complete failure only for these conditions:

- Its configured depth limit was reached.
- Rerail's parser work budget was exhausted.

These are operational failures.
They must return an adapter or budget failure.
They must not create a document-only tree or a whole-file `ERROR` node.

Forte lexer errors are recoverable source errors.
They become `ERROR` nodes in the Blade syntax tree.

## One generic node model

Every parser adapter produces this model:

```text
GenericDocument {
  source
  grammar_identity
  symbol_catalog
  root
  nodes
}

GenericNode {
  symbol
  field
  parent
  subtree_end
  start_byte
  end_byte
  flags
  classes
}
```

The generic flags are:

```text
NAMED
TRIVIA
ERROR
MISSING
```

`TRIVIA` is the tape term for a comment or other parser-designated extra node.
`ERROR` marks recovered invalid syntax.
`MISSING` marks an expected but absent symbol.

The generic `ERROR` symbol uses the reserved tape error code.
All grammar catalogs expose the same symbol.

`classes` contains optional abstract symbol memberships.
This generalizes Landing's Tree-sitter supertype table.
A grammar without abstract classes emits no class entries.

## Tree-sitter conversion

The Tree-sitter adapter walks one native tree in preorder.
It performs these mappings:

```text
kind_id       -> symbol
field_id      -> field
is_named      -> NAMED
is_extra      -> TRIVIA
is_error      -> ERROR
is_missing    -> MISSING
supertype path -> classes
```

Tree-sitter `ERROR` and `MISSING` nodes remain in the tree.
Syntax-error ranges are derived from these nodes after tape restoration.
There is no separate diagnostic table.

## Forte Blade conversion

The Forte adapter builds one Blade template tree over the original Blade source.
It then inserts error nodes before generic tape encoding.

### Error insertion

For each `LexerError`:

1. Clamp the offset to the source length.
2. Find the token that owns the offset, when one exists.
3. Read the final node that owns that token.
4. Otherwise, select the deepest node that contains the offset.
5. Break a boundary tie with the parser's token order.
6. Insert a zero-width `ERROR` child at the offset.
7. Sort it with the owner's other children by source position and insertion order.

The new node has:

```text
symbol = ERROR
field = none
flags = NAMED | ERROR
span = offset..offset
```

If Forte later supplies a reliable error range, the adapter uses that range.
The tape schema does not change.

### Missing nodes

Forte emits `SyntheticClose` tokens during recovery.
The current tree builder discards them.

The adapter can convert a synthetic close into `MISSING` when it knows the expected symbol.
For example, an omitted closing delimiter can use that delimiter's symbol at a zero-width span.

If the expected symbol is not known, use an `ERROR` node instead.
Do not create a `MISSING` node with a fabricated grammar symbol.

### Error-range derivation

`TapeDoc::syntax_error_ranges` reads `ERROR` and `MISSING` nodes for all grammars.
Blade no longer needs an additional range source.

This gives Tree-sitter and Forte the same runtime contract:

```text
recovered syntax problem -> node in the generic tree
operational parse failure -> no complete tape
```

## Embedded PHP must be a separate document

The current Blade tape appends a complete generated PHP tree as the Blade document's last child.
The PHP nodes map back onto Blade byte spans that the template tree already occupies.

This design creates overlapping sibling subtrees.
Tape restoration then disables its normal sibling-order invariant specifically for Blade.
That is not a valid generic tree representation.

The combined tree existed for one practical reason.
Rerail needs to analyze the complete PHP program that Blade executes, not isolated expression fragments.
The whole program preserves:

- Control flow formed by Blade directives.
- Variable definitions across directives and echoes.
- Scope and binding behavior.
- Calls across the compiled template.
- Bladestan and PHPStan parity.

The need for a whole PHP program does not require that program to be a child in the Blade syntax tree.

### Correct document model

A Blade source file produces two independent analysis documents:

```text
Physical file: page.blade.php

Document 0
  language: blade-template
  source: original Blade bytes
  tape: Forte Blade syntax tree

Document 1
  language: php
  source: generated PHP bytes
  tape: Tree-sitter PHP syntax tree
  origin: page.blade.php
  source_map: generated PHP -> original Blade
```

Both tapes use the same generic tape format.
Neither tape knows about the other.

The workspace owns this generic projection record:

```text
DerivedDocument {
  physical_file_id
  analysis_document_id
  language_id
  role
  source_digest
  source_map
  stable_order
}
```

This model is not Blade-specific.
Any adapter can produce a derived JavaScript, CSS, SQL, PHP, or template document.

The workspace assigns each analysis document a role:

```text
PRIMARY
EMBEDDED
GENERATED
```

An `EMBEDDED` document contains an exact slice of another source.
A `GENERATED` document contains transformed source.
The role controls routing and source mapping, not tape decoding.

A rule selects documents by grammar identity and role.
The workspace does not infer one active language for a Blade byte.
Template and generated PHP documents are parallel analysis domains.

### Source map

The existing `blade::view::Segment` data already contains the required mapping facts:

- Generated byte start and end.
- Original Blade anchor range.
- Whether bytes were copied exactly.
- Whether a generated placeholder is visible to analysis.

Move these facts into a generic source-map artifact:

```text
SourceMapSegment {
  generated_start
  generated_end
  original_start
  original_end
  mode
}
```

All ranges use byte offsets and half-open intervals.
Segments use generated-source order and do not overlap in generated space.
The workspace stores maps in physical-file coordinates.
It composes nested maps before it stores a derived document.

Modes have these meanings:

```text
COPIED    equal-length bytes map by relative offset
ANCHORED  generated bytes map to one original anchor range
HIDDEN    generated bytes have no publishable original range
```

A range mapping returns an ordered list of original ranges.
It does not silently replace a disjoint mapping with one bounding range.
Finding publication selects the rule's primary mapped capture or anchor.

A point at a segment boundary belongs to the segment on its right.
A point at generated end-of-file belongs to the final segment.
These rules also apply to zero-width `ERROR` and `MISSING` nodes.

The PHP tape uses generated PHP byte positions.
Violation publication maps captures and anchors back to Blade positions afterward.

The workspace omits roots that map only to `HIDDEN` segments from candidate selection.
The executor can still read hidden scaffolding below a visible candidate root.
The generic tape does not need an omitted-node or Blade-placeholder field.

### Validation against ast-index-spike

The `ast-index-spike` design uses the same core model at commit
`9449260fea46fa81358fc638bf95a1566d475e27`.
Its multi-language design is in `DESIGN.md` section `S070`.
The implementation is in `bench/embed_detect.py` and `bench/tape_store.py`.

That design represents a host file with these records:

- One host tape with one grammar and one source.
- One independent tape for each embedded region.
- One sparse `t_seg` row that maps the embedded tape to host byte offsets.
- No `t_seg` row for a single-language file.

An embedded tape contains only the region bytes.
Its node spans start at zero.
`resolve_match_file` adds the region start to each local match range.
Queries compile for one grammar and run only on tapes for that grammar.

Its tests establish these properties:

- An embedded query equals a standalone parse of the same region.
- A host query does not match nodes from an embedded tape.
- A match maps back to the host file and its byte range.
- Single-language files keep the original storage path.

These properties support the version-5 design.
They reject a combined kind catalog and a foreign subtree in a host tape.
They also support workspace-owned mappings between independent documents.

The Blade case requires a more general map than `t_seg`.
An embedded region is an exact source slice, so one offset is sufficient.
Generated PHP contains copied, anchored, and hidden bytes.
Therefore, Blade needs piecewise `SourceMapSegment` records.

The designs differ in one important area.
`ast-index-spike` can answer an editor-style `language_at(byte)` query for embedded regions.
That query selects the narrowest region at one byte.
It does not define routing for parallel generated documents.

The inspected nested-range support is not complete.
`language_at` selects the narrowest stored range.
However, `language_ranges` appends sorted ranges and does not split an outer range around an inner range.
The loader also detects embeds only in the host parse.
The current tests contain only disjoint Markdown regions.
Therefore, this code does not validate nested map composition.

Rerail must not use `language_at(byte)` to choose between Blade and PHP.
Both documents can provide valid facts for the same original range.
The compiled rule domain selects the analysis document.

This comparison validates the one-tree-per-tape decision.
It also adds four requirements to the workspace contract:

1. Document roles are explicit.
2. All source ranges are half-open byte ranges.
3. Nested maps are normalized to physical-file coordinates.
4. Disjoint mappings remain disjoint during publication.

## Rule compilation for Blade documents

The current Blade kind catalog combines Forte kinds and shifted PHP kinds.
Version 5 must split that catalog.

A Blade policy can compile into these matcher sets:

- Template matchers for the Blade document.
- PHP matchers for the derived PHP document.

Most existing Blade rules already use one domain:

- `directive_name`, `component`, and `attribute` use the template document.
- `function_call_expression`, `if_statement`, and `variable_name` use the PHP document.

An `any` with branches from both domains can split into two document matchers.
Their results merge in stable document order.

A conjunction that requires one root to be in both domains is not a valid single-tree rule.
The compiler must reject it or lower it as an explicit cross-document semantic relation.
It must not rely on overlapping nodes to make that conjunction appear structural.

Before migration, add an inventory for every Blade rule:

```text
template-only
php-only
splittable union
cross-document
```

No combined tape behavior can be removed until this inventory and parity suite pass.

## Version 5 binary layout

Version 5 can retain Landing's version-4 core table order.
Give each field one parser-independent meaning:

```text
1  node records
2  span deltas
3  span checkpoints
4  parent overflows
5  subtree-size overflows
6  trivia node ordinals
7  error node ordinals
8  missing node ordinals
9  symbol aliases
10 symbol-alias bytes
11 symbol overrides
12 node-class entries
```

The twelfth table uses Landing's existing `(node, supertype)` binary shape.
Version 5 names the second value an abstract class.

Rerail's old Blade syntax-error table disappears.
Its information moves into ordinary error nodes.

The header must include:

```text
TMPTAPE5
format version 5
grammar identity
grammar digest
source length
source digest
```

A v5 tape always represents one tree over one source.
There are no conditional fields by language.

## Migration from existing tapes

### Landing version 4

Landing version 4 converts directly:

- Preserve the one tree.
- Rename `extra` to trivia.
- Preserve error and missing nodes.
- Rename supertype entries to node-class entries.
- Preserve aliases and overrides.

The conversion changes the header identity to version 5.

### Rerail version 4 Tree-sitter tapes

Rerail Tree-sitter tapes convert similarly.
They have no node-class data unless the source is reparsed.
An empty class table is valid while Rerail rules reject abstract kinds.

### Rerail version 4 Blade tapes

Reparse Blade from source.
Do not mechanically convert the overlapping combined tree.

The rebuild creates:

- One Blade template v5 tape with inserted error nodes.
- One generated PHP v5 tape when PHP exists.
- One generic source map owned by the workspace.

Content-addressed tape caches make this rebuild safe.

## Rejected alternatives

### Keep the combined Blade and PHP tree

This preserves overlapping siblings and backend-specific restore logic.
Reject it.

### Rename the combined tree to a syntax view

This moves the specialization into the tape schema.
Reject it.

### Keep Blade error ranges beside the tree

The Blade adapter can create proper generic error nodes with deterministic ownership.
Reject the separate range table.

### Convert parser budget failures to whole-file errors

A resource failure is not a source syntax fact.
Reject this behavior.

### Parse each Blade expression as isolated PHP

This loses whole-template scope, control flow, bindings, and calls.
Reject it.

## Implementation order

1. Add a parser-independent `GenericDocument` builder.
2. Convert ordinary Tree-sitter tapes through that builder without byte changes.
3. Add universal generic `ERROR` and `MISSING` symbols.
4. Insert Forte lexer errors into the Blade tree.
5. Return Forte depth and budget failures instead of document-only trees.
6. Derive all syntax-error ranges from taped nodes.
7. Remove `TapeImage::syntax_errors`.
8. Expose generated PHP bytes and a generic source map from `blade::view`.
9. Encode generated PHP as its own v5 tape.
10. Add the Blade rule-domain inventory.
11. Route template and PHP matchers to their respective documents.
12. Remove the combined Blade/PHP kind catalog and overlap exception.
13. Write `TMPTAPE5` only after all old behavior has parity coverage.

The result is one tape model for every parser.
Blade specialization remains in the Blade adapter, where it belongs.
