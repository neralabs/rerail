# 0495. A rule file holds its JSON objects, regexes and patterns as programs

Status: decided (2026-11-20), by the user: "Do it now", for Q268 item 4 (the rule-file language declares no embeds).
Continues 0431 §4 (a file's programs are its grammar's declaration), 0466 (a fragment starts at a node kind, by a
hand-written parser) and 0470 (a language declares the places its strings hold code).

## Intro

A `.rr` file holds code of three other kinds:

```
.references {"siblingLinks": ["a"]}            // a JSON object
bad(s) :- kind(s, "string"), text(s, t), match("(?i)select .* from", t).   // a regex
call(c) :- c ~ `subprocess.run($CMD, shell=True)`.                        // a Python pattern
```

HTML holds JavaScript in the same way, and since 0431 an HTML file's scripts are **programs**. A JavaScript run reads
them when it says `.embedded on` (0470 step 2), and its findings land at the HTML file's lines. Until now the rule-file
language declared none (`lib/rr/embeds.rr`): no regex grammar existed, no pattern grammar either, and no embedder read
a JSON object.

There are 6,721 `match` regexes in 642 of the repository's 1,693 rule files, 212 Python patterns, and 13 JSON
objects. So a regex rule set (look-around, which the engine's regex dialect refuses; a catastrophic repetition) can
check every rule file's regexes, and a JSON or Python rule set can check theirs.

## Decision

### 1. JSON objects (`via: json-object`)

A `json_object` leaf of the rule file's tape is a JSON program. Its text is the leaf's bytes, byte for byte, so its
nodes stand at the file's bytes. It is a program per place, at the leaf.

### 2. Regexes (`via: match-regex`), and the regex grammar

- **The grammar.** `regex` is a registered language: one dialect, a native grammar (`native:regex`) read by a
  hand-written parser (`literal::regex`). It reads the Rust `regex` crate's syntax, which is the dialect the engine
  matches with. Its tree: a `regex` root; `alternation`, `concatenation`, `group` (with its `name` where it is named),
  `repetition` (`*`, `+`, `?`, `{n,m}`, lazy or not), `class` (with its `range`s and `class_name`s), `literal`, `dot`,
  `anchor`, `escape`, `flags`. A look-around, a back-reference or another construct the dialect refuses is an
  `unsupported` node over its bytes. A text the parser cannot read at all is a `regex` root over one `error` node, its value the message (the
  literal grammars' convention).
- **The place.** The first argument of a `match(re, s)` atom, when it is a plain string, holds a regex program. Its
  text is the string's decoded value. Its segments map each decoded character to the bytes it was written with, so an
  escaped character (`\\.`) stands at its two bytes.

### 3. Patterns (`via: pattern`), and a language's pattern fragment

- **The fragment.** A language may register a fragment that starts at a `pattern` (0466). Python's lean parser reads
  one: a module whose metavariables (`$X`, `$$$X`, `$_`) are names. Its tape is the pattern's tree as Python reads
  it, so a Python rule set reads a pattern as it reads code.
- **The place.** The backtick `code_literal` of a `pattern_match` (`n ~ \`…\``) holds a pattern program in the file's
  `.language`, where that language registers a pattern fragment. Otherwise it holds none. Its text is the literal's
  content, byte for byte.

### What does not change

- A rule that reads a regex as a value still parses it as any string: `parsed(s, "regex", r)`.
- A run of another language reads a rule file for these programs only under `.embedded on`.

## Requirements (each with its test before the code)

- R1. A rule file's JSON objects are JSON programs. A JSON rule set under `.embedded on` finds a JSON object's
  member, at the rule file's line.
- R2. `regex` is a registered language with a native grammar and a tape tag. It parses the Rust dialect's constructs
  into the kinds above, and refuses look-around as an `unsupported` node over its bytes.
- R3. A `match`'s first string is a regex program, placed at the string's bytes through its escapes. A regex rule set
  under `.embedded on` finds a look-around at the rule file's line.
- R4. Python registers a `pattern` fragment: `$X`, `$$$X` and `$_` read as names, and the tree is Python's.
- R5. A pattern in a `.language python` rule file is a Python program. A Python rule set under `.embedded on` finds a
  call in a pattern, at the rule file's line. A pattern in a file of a language with no pattern fragment holds none.
- R6. `lib/rr/embeds.rr` describes the three programs, and the rule-file run's own findings do not change.

## Tests

- R1: `a_rule_files_json_objects_are_json_programs`.
- R2: `the_regex_grammar_reads_the_rust_dialect`.
  `every_rule_files_regex_reads_in_the_regex_grammar`: each of the repository's `match` regexes that the `regex` crate
  compiles (over 5,000) reads with no `error` node.
- R3: `a_match_regex_is_a_regex_program_at_its_bytes`.
- R4: `a_python_pattern_reads_metavariables_as_names`.
- R5: `a_rule_files_patterns_are_programs_of_its_language`.
- R6: `a_rule_about_rules_runs_over_rule_files` (unchanged).
