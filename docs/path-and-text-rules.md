# Writing path and text rules

A code rule doesn't have to parse code. Three queries work on any text file, in any language or none:

| You want to say | Query | Reported at |
|---|---|---|
| "this text should not appear in these files" | `{ "regex": { "source": "file.content", "pattern": "…" } }` | the match: `path:line:column` |
| "a file with this kind of path should not exist" | `{ "regex": { "source": "file.path", "pattern": "…" } }` | the file: `path`, no line |
| "the files this rule's scope names should not exist" | `{ "file": {} }` | the file: `path`, no line |

For syntax queries (`ast`), see [Writing a code rule](../skills/rerail-create/references/code-rules.md). For the full
grammar, see the [DSL reference](code-rule-dsl-reference.html#queries).

- [Which files these rules read](#which-files-these-rules-read)
- [Text in files](#text-in-files)
- [Where files live](#where-files-live)
- [Files that should not exist](#files-that-should-not-exist)
- [Test it](#test-it)

## Which files these rules read

A rule with a `regex` or `file` query reads every text file in its `scope`:

- source files of the languages Rerail parses (Python, TypeScript, JavaScript, PHP, Blade, Rust, SQL, Vue);
- files Rerail reads as text by suffix or name: `.json`, `.yaml`, `.yml`, `.toml`, `.md`, `.mdx`, `.rst`, `.txt`, `.sh`,
  `.bash`, `.zsh`, `.html`, `.css`, `.scss`, `.gitignore`, `.npmignore`;
- any other file without a NUL byte, such as `Justfile`, `LICENSE`, `NOTICE`, `.gitattributes` or `Cargo.lock`.

A file with a NUL byte is binary, and no rule reads it. `rerail rules test` counts the binary files in a rule's scope, so you
can see what it passed over. No scan reads `.git`, `node_modules`, `dist`, `build`, `target`, `vendor` and the other
folders listed in the [reference](code-rule-dsl-reference.html#files-read), or files Git ignores while the repository
follows `.gitignore`.

A rule made only of `ast` queries reads only its language's files, however wide its scope.

## Text in files

A `file.content` regex reports each match at its line and column. This one catches an old folder name anywhere it is
still written down, `Justfile` and lock files included:

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:no-old-tools-path",
    "revision": 1,
    "mode": "steer",
    "description": "Refer to scripts/ instead of the old tools/ folder, which was renamed.",
    "scope": { "include": ["**/*"], "exclude": ["CHANGELOG.md"] },
    "clauses": [
      {
        "id": "old-path",
        "forbid": { "regex": { "source": "file.content", "pattern": "\\btools/" } },
        "violation": { "code": "OLD_TOOLS_PATH", "anchor": "witness.span", "message": "The tools/ folder is now scripts/: update this reference." }
      }
    ]
  },
  "cases": {
    "caught": [{ "path": "Justfile", "content": "lint:\n    node tools/lint.mjs\n" }],
    "passed": [
      { "path": "Justfile", "content": "lint:\n    node scripts/lint.mjs\n" },
      { "path": "Justfile", "content": "lint:\n    node devtools/lint.mjs\n" },
      { "path": "CHANGELOG.md", "content": "- tools/ is now scripts/\n" }
    ]
  }
}
```

The regex dialect is Rust's: no look-around and no backreferences. `(?m)` makes `^` and `$` match at each line. In JSON,
write every backslash twice. Exceptions that a pattern cannot express go in `scope.exclude`.

## Where files live

A `file.path` regex reads the file's repository-relative path, with `/` between folders. A match is about the file, not a
place in it, so the violation names the path with no line, once per file however often the pattern matches. Named
groups (`(?<NAME>…)`) still fill in `$NAME` in the message.

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:kebab-case-components",
    "revision": 1,
    "mode": "steer",
    "description": "Name component files in kebab-case.",
    "scope": { "include": ["src/components/**/*.tsx"], "exclude": [] },
    "clauses": [
      {
        "id": "case",
        "forbid": { "regex": { "source": "file.path", "pattern": "(?<NAME>[^/]*[A-Z_][^/]*)\\.tsx$" } },
        "violation": { "code": "COMPONENT_FILE_CASE", "anchor": "witness.span", "message": "Rename $NAME.tsx to kebab-case, for example user-card.tsx." }
      }
    ]
  },
  "cases": {
    "caught": [{ "path": "src/components/UserCard.tsx", "content": "export {};\n" }],
    "passed": [
      { "path": "src/components/user-card.tsx", "content": "export {};\n" },
      { "path": "src/components/Admin/user-table.tsx", "content": "export {};\n" }
    ]
  }
}
```

A path match takes no `fix` and no `capture.NAME` anchor. Anchor it at `file.start` only if a tool you compare with
reports such files at line 1.

## Files that should not exist

When the rule is "these files should not be here", the scope says which files, and the clause needs no pattern: a
`file` query matches every file in scope. Exceptions go in `exclude`.

```json draft
{
  "kind": "code",
  "rule": {
    "format": "rerail.policy/v1",
    "id": "local:pack-json-placement",
    "revision": 1,
    "mode": "steer",
    "description": "Keep each pack's pack.json at the top of its pack folder.",
    "scope": { "include": ["**/pack.json"], "exclude": ["packs/*/pack.json"] },
    "clauses": [
      {
        "id": "misplaced",
        "forbid": { "file": {} },
        "violation": { "code": "MISPLACED_PACK_JSON", "anchor": "witness.span", "message": "Move this pack.json to the top of its pack folder, packs/<name>/pack.json." }
      }
    ]
  },
  "cases": {
    "caught": [{ "path": "packs/python/rules/pack.json", "content": "{}\n" }],
    "passed": [
      { "path": "packs/python/pack.json", "content": "{}\n" },
      { "path": "packs/python/rules/rule.json", "content": "{}\n" }
    ]
  }
}
```

A `file` query reads no text, so a case's `content` can be anything.

A session's default audit scope (`ai-output`) tells the agent only about what its own edit wrote. A match on a path, or
of a `file` query, is about the whole file, so the edit that creates the file (a write, a copy or a move) is told; an
edit to a file that was already there is not. The `touched-files` and `repository` scopes report every such file.

## Test it

Each example above is a complete draft: the rule, plus the cases it must catch and let pass. Save one to a file outside
the repository and run:

```sh
rerail rules test draft.json          # validate, run the cases, list the matches in this repository
rerail rules test draft.json --json   # the same, with every match and every file it could not check
rerail rules add draft.json           # store it in your library and add it to this repository
rerail rules test --help              # the draft format
```

The text report shows the first five matches. `--json` lists every one, so you can compare the rule with another tool's
results in one run. A path or `file` match has no `line` or `column`.

The repository pass of `rules test` shows the coding agent lines of your files, so it leaves out hidden files such as
`.gitattributes`, files named like secrets, and files Git ignores, and counts them without names
([Reference](reference.md#scan-and-settings-controls)). Show that a rule about such files works with its cases.
