# Parser generators: the tree section as a grammar

Status: decided (user, 2026-11-07: "yes, please make that change"). Decision 0085. It
replaces the tree section's start and end rules (`after`, `before`, `also`, `parent`, `with`,
`field … of`, `supertype … of`) and tier 2's dataflow of docs 14 to 16.

## Intro

Tier 2 is simdjson's stage 2: a state machine over the token stream. Simdjson's states are its
grammar: after `{` only a key or `}` may come, after a value only `,` or `}`. The old tree
section could not give those states. It said where nodes start and end, not what may follow
what, so a file like `[1 2]` got a tape where tree-sitter finds an error.

So the tree section now states each node's content, as tree-sitter's `grammar.js` does. The
compiler turns it into one small automaton per nesting level, and a stack of return states.
That automaton is the state machine, the grammar check, and the source of every field and
supertype.

## 1. The syntax

```
tree json {
  node document               = value
  node object nested          = '{' [pair (',' pair)*] '}'
  node pair                   = key: string ':' value: value
  node array nested           = '[' [value (',' value)*] ']'
  node string                 = '"' (string_content | escape_sequence)* '"'
  choice value supertype _value = object | array | string | number | true | false | null
}
```

| form | meaning |
|---|---|
| `node K [nested] = E` | A node of kind `K` spans what `E` matches. The first `node` is the root. |
| `choice N [supertype S] = A \| B …` | No node of its own. Each alternative's node gets supertype `S`. |
| `'x'`, `name` | A token: a literal is a keyword's kind; a name a token kind, a node or a choice. |
| `f: X` | The node `X` gives gets field `f`. |
| `A B`, `A \| B`, `[A]`, `A*`, `(A)` | sequence, alternative, optional, repetition, grouping. |

- **`nested`** marks a node that may contain itself. Its content runs on a stack frame; every
  other node is inlined into the level that uses it. A non-nested node may not refer to
  itself, directly or through others.
- **Shared rules.** The compiler also gives a level of its own to a node or a choice that is
  inlined in several places, when a call saves enough states. Inlined, each place has a copy
  of the rule's inner states: those it may go on from. Called, the level has them once, and
  each use pays a push and a return. The states saved are the inner positions, once instead
  of at each use, less a state per call:

  ```
  saved = (uses - 1) * inner - uses       # shared when saved >= 8
  ```

  A choice of single tokens has no inner position: each place needs its states either way, so
  it stays inlined (JSON's `value`). So does JSON's `pair`: two uses save three states. A rule
  that may match no token, or whose first token merges into the leaf before, stays inlined
  (a call takes a token of its own), as do the rules a level that refuses a mark uses.

  `.rr`'s expressions and aggregates are used in many places. With every rule inlined, its
  machine had 858 states and 8,370 arms; shared, 438 and 4,014.

  States of different levels that act alike stay apart, measured and not taken. Merging them
  (a level's identity left out of the merge, levels that share a state joined) takes `.rr`
  from 438 states to 284 and from 4,014 arms to 2,881. But tier 2's words do not change
  (12,915 either way): its words are merged where they act alike (`merge_words`), so those
  states were one copy of code already.

- **Shared arms.** Tier 2's code also has one copy of each arm body. An arm is a run of
  plain words (its span, keyword check and acts) ending in a `GOTO`. Runs with the same words
  are one body, lowered once. Each place enters it with the index of its own target, and the
  body ends in a jump table on that index. A refusal inside the body names the place's own
  word: the place also sets how far its words are from the body's.

  The shape check reads the control-flow graph, where a body's jump reaches every place's
  target, not only its own. A cycle there that passes no `TOKEN` is refused, even though it
  cannot happen. So places share only while the word graph, with each body's jump to all its
  targets, has no such cycle; a place whose false edge closes one keeps its own body.

  On `.rr`, 1,620 runs become 379 bodies: 6,090 words lowered as 1,900. Tier 2's machine
  code goes from 261K to 246K instructions, while its stack loads and stores go from 66K to
  98K. The words were not most of the code: tier 2 keeps about 20 state variables in SSA,
  and each block where paths meet carries all of them. A shared body is such a block for
  hundreds of places.

- **State in memory where few words touch it.** Tier 2 keeps in SSA only the state every
  token uses: the cursors, the token's start, end and limit, the node count, the open and
  return depths. The supertype count, pending extras, wraps, the moved node, dropped records
  and a shared arm's target and shift are slots of the stack frame, read and written where
  used. On `.rr`, with shared arms, tier 2 goes from 246K instructions to 202K, and its stack
  loads and stores from 98K to 43K (the slots' own included): both below the 261K and 66K
  before shared arms.
- **Every token is a leaf node,** as before; `skip` bytes are in none.
- The root spans the file from its first token to the file's end, as tree-sitter's does
  (trailing whitespace included).

## 2. The automaton

For each level (the root and each nested node) the compiler builds a DFA over the tokens:

1. **Inline** every non-nested node and choice into the level's expression. A use of a nested
   node stays a symbol: a call.
2. **Glushkov's construction:** a state per position, merged where states act alike.
3. **Actions on each transition**, from the edge it crosses, in order:
   - close the nodes the previous token was the last of (innermost first), at its end;
   - open the nodes whose first token this is (outermost first), each with its field and
     supertypes from the path to it;
   - the token's own leaf node.
4. **A call** to a nested node is a transition on the node's first token: open the node, push
   the return state, enter the node's level. After the node's last token the level pops at
   once; the return state's next arm closes what was waiting on it (a pair whose value it was).
5. **Refusals:** a token with no transition, or the end of the file anywhere but an accepting
   root state with the stack empty, sends the file to tree-sitter.

The grammar must be deterministic on one token of lookahead (LL(1) per level). The compiler
refuses a grammar that is not.

## 3. JSON's states

The compiler gives JSON 19 states (`json_tier2_states_print_as_designed` pins them whole):

| state | level | after | takes | simdjson |
|---|---|---|---|---|
| 0 | document | the start | a value | first `{` `[`, or `visit_root_primitive` |
| 2 | document | a value | the end | `document_end` |
| 4, 10 | document | `"`, the string | content; the end | `parse_string` |
| 1 | object | `{` | `"` (a key) or `}` | `object_begin` |
| 5, 11 | object | the key's `"`, the key | content; `:` | `parse_string`, `object_field` |
| 14 | object | `:` | a value | `object_field` |
| 15, 18 | object | a value, a string value | `,` or `}` | `object_continue` |
| 16 | object | a value's `"` | content, escape, `"` | `parse_string` |
| 17 | object | `,` | `"` (a key) | `object_continue` |
| 3 | array | `[` | a value or `]` | `array_begin` |
| 7, 13 | array | a value, a string value | `,` or `]` | `array_continue` |
| 8 | array | `"` | content, escape, `"` | `parse_string` |
| 12 | array | `,` | a value | `array_value` |
| 6, 9 | object, array | `}`, `]` | (pops) | `scope_end` |

A string is inlined into each place it is used, so each has its own inner state, and a state
after its closing `"` that closes it on the next token. A pop returns to 2, 7 or 15: simdjson's
`scope_end`, which branches on `is_array[depth]`.

Three arms, as `rir` prints them (kinds: 17 object, 18 pair, 20 string, 16 `_value`; field 1
`key`, 2 `value`):

```
arm 14 token 1 to 1 push 15 do open 17 f 2 s 16 leaf    # `{` after `:`: a call
arm 17 token 7 to 5 do open 18 open 20 f 1 leaf         # `"` after `,`: a pair and its key
arm 18 token 2 to 17 do close close leaf                # `,` after a string value: closes
                                                        # the string and the pair
```

## 4. The stream and the kinds

- **The stream** is two of tier 1's extracts, as simdjson's structural indexes: every
  token's first byte, and the last byte of each token whose row does not give its end (a
  one-byte token ends after its byte; a keyword-only row's word gives its length). The
  compiler knows which rows take a last byte, so an arm's `SPAN` reads the next one, never
  searches (decision 0087).
- **The token's kind** is its first byte, as simdjson's `switch (*value)`: a `SWITCH` on it.
  A word starting `t`, `f` or `n` is switched on byte by byte and then compared once with
  `true`, `false`, `null` (`WORD`, one load and compare, its length from its last byte); an
  escape's second byte picks its kind or refuses.
- **A number's syntax** is tier 2's, as simdjson's `parse_number` (decision 0086): `check number = G`
  is a grammar over the token's bytes. The compiler makes it a byte state machine (8 states,
  `parse_number`'s steps), then straight-line operations over the token's window, a `fill`
  per digit run; the number's arm runs them. No loop over the bytes. Tier 1 finds only the word; `true`, `false` and `null` retype it, and any
  other word is a number the check refuses.

## 5. What it replaces

- **Rows' conditions go:** `after`, `before`, `also`, `parent`, `end with`, `end` by kind, the
  code facts. The grammar gives every field and supertype as a constant of a transition. The
  rows keep the tokens: their masks, keywords, and the stream.
- **`nested max 13` goes:** the bound was the stack effect's width. The stack is memory, sized
  by the file's opens, as simdjson's.
- **Tier 2's dataflow (docs 14 to 16, phases 3 and 4) is redone for Metal** (phase 6), from
  the automaton: the carry is the depth and its low point; closes find their opens by nearest
  smaller value (doc 01 §3, `metal_nearest`); within a level, the transitions compose as state
  functions (phase 2's `StateFn`, wider), with each transition's node count. No backward value:
  every rule reads forward. The monoids, the plan's stores and the barrier verifier stay.

## 6. The reference

`tools/bitlex` parses the token stream with the grammar, one token at a time: the reference
nodes and refusals, the evaluator's tree. The engine's tapes are checked against tree-sitter,
as before.
