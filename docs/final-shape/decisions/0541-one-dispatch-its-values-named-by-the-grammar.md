# 0541: One dispatch a rule, its values named by the grammar

Status: done by the implementer. The two items are the user's (2026-11-20, "address each of these in turn", the
first two of 0526's "Left"): "Three nested dispatch loops instead of one … The lean parser has one match label with
six labels", and "Label names say where, not what. Names like ObjectBegin would have to come from the grammar's
rules". How each is done is the implementer's choice. Follows 0526. Page 40, step 45.

## The decision

1. **One dispatch for a rule's outermost loop entered at two places, the loops within it split**
   (`Shape::threaded`, in `rerail-clif`). Where splitting nodes alone needs more copies than the graph has nodes,
   as before, a dispatch is made for the outermost such loops only (`dispatched(.., outer: true)`), and the loops
   within them are split again:
   - an edge back into a loop entered at two places (`two_entries`) whose target branches goes to the dispatch
     over it in the dominator tree (`dispatch_over`), with a new value, as the lean parser's
     `break Label::ArrayValue` from its `ScopeEnd`;
   - any other such edge is given a copy of its target, as before. An entry that goes on to one place (a `TOKEN`
     before a switch) is copied, not sent through the dispatch.
   - The arms that only jump on are threaded after the split too (`thread`).
   - Where the split still passes the budget, every loop entered at two places gets its own dispatch, as before.
2. **A dispatch's value is named by what its place does in the grammar** (`rust_stream::meaning`), from the
   grammar's names of its kinds and fields. These now reach the IR: the grammar's compile writes `kindname #K
   "NAME"` and `fieldname F "NAME"` rows (`bitlex_masks::tree_section`), the assembler reads them into
   `Words::kind_names` and `field_names`, and `native_ir::lower` copies them to `Structure`. No word reads them.
   - A rule's first word: the kind every call of it opens before it, and `Begin` (`ObjectBegin`).
   - A `RETURN`: its rule's kind and `End`; `End` alone where rules of two kinds share it (JSON's).
   - A switch, or the `TOKEN` before one: its rule's kind, then what every arm that makes a node makes: their
     supertype, else their field, else their kind (`ArrayValue`: every arm's node is a `_value`).
   - Where the names do not tell, the place's name, as 0526's (`L::Token32`). A second place of one meaning gets its
     word's number after the name.

For JSON the rule has one `match d`, with four values: `ObjectBegin`, `ArrayBegin`, `ArrayValue`, `End`. The
object's loop (`'switch_155`) and the array's (`'switch_98`) are each entered once; a primitive value's `,` goes
back to its loop's header directly. A container's close, then the closes after it in an array, is a loop of its own
(`'span_40_1`), as the lean parser's `ScopeEnd`. The lean parser's `ObjectKey` and `DocumentEnd` are direct jumps
here, not values.

The rules for naming, and for choosing between a dispatch and a copy, are the implementer's. `End` for a shared
return, where the lean parser (and simdjson) say `ScopeEnd`, is too.

## Checks

- `json_tests::simdjson_forms_rust_trees_are_the_jits`: one `match d {` in the format 15 source and one in the
  format 16 one; the enum's values `ObjectBegin`, `ArrayBegin`, `ArrayValue`, `End`; the tapes equal the JIT's,
  as before.
- The parser tests (85, all but the benches, Metal and the corpus), which include
  `aot_tests::grammars_rust_tapes_are_the_jits`: every grammar's Rust writes the JIT's tapes.
- `rerail-compiler`'s tests (127), which read and write the program text.
- `rerail-clif`'s tests (14).
- `rerail-rr-aot` (generated, on a farm host): 15 pass, 6 ignored.

## Not checked

- The other grammars' dispatches: how many each rule has now, and what their values are named. The AOT test checks
  their tapes, not their shape.
- Whether `clif_rust` (which goes through `Shape::of`) now writes a different shape for any function: its tests
  pass.
- Nothing about speed was measured.
