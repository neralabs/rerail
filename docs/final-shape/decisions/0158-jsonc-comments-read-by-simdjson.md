# 0158: A JSON file's comments are read by simdjson, in a second read

Status: accepted (user, 2026-11-13: "take a look at O-jsonc-simdjson and figure out what can be done
(even as a second path when we detect a syntax error caused by a comment) to deal with those")

## Intro

simdjson reads strict JSON. A file with comments (`tsconfig.json`, editor settings) was refused and read
by tree-sitter-json, at about 30 times the cost (O-jsonc-simdjson).

tree-sitter-json's grammar has comments: `comment` is an extra, a node the grammar allows between any
two tokens. Where tree-sitter puts one follows a fixed rule. So simdjson's tape can hold comments too,
and be tree-sitter-json's byte for byte.

Trailing commas are not in the grammar. tree-sitter-json recovers from one with an `ERROR` node or a
missing `number`, depending on the context (see Results). That cannot be reproduced exactly, so those
files stay with tree-sitter-json.

```
a JSON file -> the strict read (0134, 0148) -> the tape
                 refused, and the file has `//` or `/*`
            -> the JSONC read: comments blanked for stage 1, written as nodes by stage 2 -> the tape
                 refused (a trailing comma, invalid)
            -> tree-sitter-json
```

## Prior work consulted

- **ast-index-spike indexes:** the spike had no JSON parser of its own to compare with.
- **0134** (JSON is read with simdjson) and **0148** (parsers routed by what the host runs).
  **Extended:** the same kernel gains a JSONC path, and the router's choice is unchanged.
- **Lessons ledger #10** (measure the code that runs). **Agrees:** the JSONC tapes are compared with
  tree-sitter-json's tapes, as the engine encodes them, byte for byte.
- **String matching is chunked** (the standing rule: no per-byte scans). **Agrees:** comments are found a
  64-byte block at a time, by simdjson's own string scanner.
- **The loose findings contract** (findings are a set of keys, with no exact-shape parity required).
  **Not relied on:** comments are matched exactly. Whether trailing commas may depart from tree-sitter
  is left to the user (Open).

## Decision

1. **The strict read comes first,** unchanged. Only a file it refuses that contains `//` or `/*` is read
   again as JSONC (`json_parser::simdjson::encode_with`). A strict file costs what it did, and a refused
   file without a comment goes straight to tree-sitter-json.
2. **Stage 1 finds the comments** (`index_and_hash<H, true>`):
   - Each 64-byte block is scanned by simdjson's scanner. A `/` outside a string starts a comment: `//`
     runs to the line's end (its `\n` not included, a `\r` included), and `/*` runs to the next `*/`.
   - Each comment is recorded (start, end) and blanked in the kernel's own copy of the source. The block
     is then scanned again from the scanner's state at its start, so a quote inside a comment opens no
     string.
   - A `/` that starts no comment, or a `/*` never closed, refuses the file.
   - The digest is of the source as given, read from the caller's bytes, not the blanked copy.
3. **Stage 2 writes each comment** (`walk<…, true>`) as a `comment` leaf with the extra flag. Its parent
   is the lowest node holding the tokens on either side of it, which is where tree-sitter-json puts an
   extra. In the walk that means a comment is written where the walk stands when it reads the token after
   the comment:
   - before a key, or after a `,` in an object: the object's child;
   - after a key, or after its `:`: the pair's child;
   - after a pair's value: the object's child, since the pair has ended;
   - in an array: the array's child; around the value: the document's child;
   - in an empty object or array: that object's or array's child.

   The document starts at its first token, a leading comment included.
4. **The strict path's code is untouched.** The JSONC path is a separate instantiation of the same
   template (`C`), so the strict walk has no comment checks.

## Results

Every hand-written case equals tree-sitter-json's tape, byte for byte
(`json_tapes_from_simdjson_are_tree_sitters`). The cases cover a comment at every place in an object and
an array, comments in empty containers, `\r\n`, a byte order mark, `/*` inside a string, a quote inside a
comment, and a tsconfig-shaped file. The crate's test checks each kernel this CPU runs, including
westmere under Rosetta.

Over the `.json` files of two worktrees' `node_modules` (2,099 files; `simdjson_tapes_over_files`):

| | files |
|---|---:|
| strict, equal to tree-sitter-json's tapes | 2,035 |
| JSONC (comments), equal to tree-sitter-json's tapes | 13 |
| refused, so read by tree-sitter-json (trailing commas) | 51 |
| tapes that differ | 0 |

In instructions, on arm64, over those files (`tape_cost`):

| | a node |
|---|---:|
| the strict tape (2.2 M nodes, 28.9 MB) | 127 |
| the 13 JSONC files: the strict read refused, then the JSONC read | 523 |
| the JSONC read alone | 351 |
| tree-sitter-json's tape (0134, the repository's files) | about 18,000 |

The JSONC files are small (14.8 KB in all), so per-file setup weighs more in their cost. The strict path
is unchanged: the repository's tape is 751 M instructions, against 757 M before.

**Trailing commas**, as tree-sitter-json reads them:
- `[1, 2,]`: a missing `number` (zero width) before `]`;
- `[[1,],]`: an `ERROR` node holding the inner `,`, and another for the outer one;
- `["x",]` and `[{…},]`: an `ERROR` node;
- `{"a": 1,}`: an `ERROR` node, an extra of the object.

Which form it takes depends on the context, by its error-recovery costs, not on a rule.

## Open

- **O-jsonc-trailing-commas** (in O-jsonc-simdjson): trailing commas, the more common case in npm's
  `tsconfig.json` files (22 with trailing commas only, 6 with comments only, 3 with both, out of 1,000
  files). Exact parity with tree-sitter's recovery is not reachable. The options are the user's: keep
  them on tree-sitter-json, or give such a file the tape its JSONC meaning has (as TypeScript reads it,
  the comma ignored), a departure from tree-sitter's error tree.
