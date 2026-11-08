# Parser generators: the tree section as a grammar

Status: decided (user, 2026-11-07: "yes, please make that change"). Decision 0051. It
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

- **The stream** is tier 1's one extraction: every token start, and the last byte of each
  token whose end the next start does not give (a word before a space). The compiler knows
  which token kinds have an end entry, so a transition reads one or two entries, never searches.
- **The token's kind** is its row's, or from its bytes: a punctuation byte is its kind; a word
  starting `t`, `f` or `n` is compared once with `true`, `false`, `null` (one 32-bit compare,
  and its length from the end entry); an escape's second byte picks its kind or refuses.
- **A number's syntax** is tier 2's, as simdjson's `parse_number` (decision 0052): `check number = G`
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
