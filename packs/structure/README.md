# Rerail rule pack: Structure

Rules that keep a codebase's shape as coding agents grow it: files stay short, imports point down the layers, exports
are used and tested, and file names say what the file holds. They are general: scoped by language globs and by
folder names most TypeScript and JavaScript repositories share (`lib`, `utils`, `services`, `components/ui`, `app`),
not by any one repository's paths. Thresholds were chosen to flag roughly the top 2 to 3% of open-source repositories.

Set-up offers the pack to a repository with TypeScript or JavaScript source; it is a `repository` pack.

The pack is private (`"private": true` in its `package.json`) while it is a proposal: `rerail packs add ./packs/structure`
installs it from this checkout. Two more rules, a per-directory file limit and a duplicate function body check, are
drafts in [candidates/README.md](candidates/README.md): both count across files, and a pack example is one file.

| Group | Rule | What it flags | Mode |
|---|---|---|---|
| Size | `structure-file-line-limit` | A source file over 500 lines | Steer |
| Layering | `structure-layers-depend-downward` | Shared code importing a page, route, server action or component; a UI primitive importing feature code; a service importing Next.js | Steer |
| Tests | `structure-untested-exports` | An exported TypeScript function no test file imports | Steer |
| Dead code | `structure-unused-exports` | An export nothing imports and its own file never uses | Log |
| Naming | `structure-vague-file-names` | A file named as a backup or copy (`x copy.ts`, `x.bak`, `x (2).ts`) or a catch-all (`misc.ts`, `stuff/`, `untitled.ts`) | Steer |

## Rule details

### `structure-file-line-limit`

Counts every line of the file, blank lines and comments included, and flags 501 or more. Tests, stories, scripts,
config, migrations, locales, icons and generated code are out of scope, as is any file with an `@generated`,
`do not edit` or `auto-generated` comment. TypeScript, JavaScript and Python. The message tells the agent to put new
code in a new module and not to split existing code unasked.

### `structure-layers-depend-downward`

Five clauses, each on a value import or re-export resolved through an alias (`@/`, `~/`, `#/`) or a parent-relative
path (`../`); `import type` is always allowed, and asset imports (images, CSS, JSON, Markdown) are ignored.

- Shared code (`lib`, `libs`, `utils`, `helpers`, `shared`, `core`, `domain`, `db`, `database`, `models`, `services`)
  imports no `app`, `pages` or `routes` module, and no server action (`actions`, `server/actions`).
- Non-UI shared code imports no `components` module (`.tsx` files and `hooks` folders are exempt).
- A UI primitive (`components/ui`) imports no page, route, feature, module, action, service or database code.
- A service (`services/`) imports nothing from `next`, so it runs in tests.

Same-folder imports (`./x`) and bare package aliases other than the ones above are not resolved, so a layer crossing
written that way is missed.

### `structure-untested-exports`

Pairs each exported function (declarations and `const` arrow or function expressions) with test files that import it
by name, re-export it, or import its module by namespace or dynamically. Default exports, overload signatures and files
with a `"use server"` or `"use client"` directive are not checked; components, actions, entry files, config and
scripts are out of scope. A name imported by any test counts, so two exports with the same name in different modules
satisfy each other.

### `structure-unused-exports`

Flags an export (function, constant, class or type) that no other file imports and its own file never uses, so it is
dead code rather than an unneeded `export` keyword. Default exports, Drizzle table schemas, types derived with
`typeof`, overload signatures, and entry, route, index, test, config and generated files are not checked. Matching is
by name and module stem, so an import of the same name from another module hides a dead export. Starts in Log: a
caller often lands in the next edit.

### `structure-vague-file-names`

Two clauses on the file path. Leftover copies: `.bak`, `.orig`, `.old`, `.backup`, `copy`, `copy of`, ` (2)`,
`final-v2` and similar suffixes. Catch-alls: `misc`, `miscellaneous`, `untitled`, and `stuff` as a file or folder name
or word. `misc/` folders, version numbers and words like `new` or `temp` pass.
