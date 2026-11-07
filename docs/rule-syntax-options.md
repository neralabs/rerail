# Rule syntax options

Status: exploration, 2026-11-03. Nothing here is implemented.

Today a code rule is a `rerail.policy/v1` JSON document. This note compares other ways to write the same rules, using one working example and one data-layer rule. The data-layer pack is the real test: its 142 checks have to be writable in whatever syntax we pick.

## What the evidence says

- **Size comes from duplication, not from JSON.** The 70 data-layer candidate ports (`Data-layer-rule-expressivity/packs/data-layer/candidates/rules/`) total 8.3 MB. `serverplan-steps` alone is 1.2 MB in 22 clauses: the same 10 KB scope matcher appears three times, and authored SQL strings are inlined in the rule. Rules have no way to share helpers with each other, so each one carries its own copy. YAML would shrink the punctuation but not the copies.
- **The prior art is YAML with SQL inside it.** nera-data-layer PR 2539 (`scripts/lint_specs/*.yaml`, `nera.linter/v1`) moved about 100 linters to YAML pipelines: `from`, `join`, `where`, `project`, `classifiers`, `ratchets`. Its own design (`docs/design/LINTER_EXPRESSION_FACT_QUERY_DESIGN.md`, "YAML Sufficiency Boundary") forbids raw SQL, but every `where.expr` is an SQL expression string. For example, `function_level_imports` spells the regex `^l\d{3,4}_[A-Za-z]` as 20 `SUBSTR` comparisons. The lesson: if YAML has no expression language of its own, string-embedded SQL leaks in anyway.
- **Datalog is already the decided underlying format.** wolfeel `docs/final-shape/03-architecture.md` §12 ("Rerail logic") sets out one AST with two concrete syntaxes: Datalog text for facts and inference, and YAML/JSON for matcher-style lint rules. Both embed matchers as an `ast(...)` atom, and neither carries SQL. The Typing-support `.rdl` programs (`calls.rdl`, `workload.rdl`) are working examples of that dialect: `.decl`, `$X` variables, `.kindset`, `nearest`, `!`, `count { }` and `@cat`.
- **TypeScript import resolution is missing.** The DSL reference says "TypeScript import resolution is out of scope" for `resolves.import`. That is why the example rule hand-builds the import walk through `binding.site` and `inside: import_statement`. Some of the example's verbosity is a missing fact, not a syntax problem.

## The working example

`local:no-explicitly-nested-routers` reports a react-router router element nested inside another one. The walk up from the inner router stops at function boundaries, JSX attributes and composite (capitalised) elements. Host elements (`<div>`) and fragments (`<>`) are transparent.

The JSON is 164 lines. The variants below say the same thing unless marked otherwise.

### A1. YAML, transliterated (87 lines)

A mechanical conversion of the JSON. It needs no design work and halves the line count, but the tree is just as deep:

```yaml
utils:
  router-name:
    kind: identifier
    resolves:
      binding:
        form: import
        site:
          kind: import_specifier
          has:
            field: name
            regex: "^(BrowserRouter|HashRouter|MemoryRouter|Router|RouterProvider|HistoryRouter|unstable_HistoryRouter|StaticRouter)$"
          inside:
            kind: import_statement
            stopBy: end
            has:
              field: source
              kind: string
              any:
                - value: {eq: react-router}
                - value: {eq: react-router-dom}
                # ... four more
```

### A2. YAML with sugar (36 lines)

These additions are surface sugar. Each one desugars to an existing construct, except `resolves.import` for TypeScript, which needs the missing fact.

- `kind: [a, b]` means `any` over kinds.
- `tag:` means `open_tag.name` or `name`, whichever the element has.
- `in:` means a set membership test.
- `let:` defines named constant sets.
- `resolves.import: {module, name}` for TypeScript.
- Defaults are filled in: `format`, and `anchor: witness.span`.

```yaml
id: local:no-explicitly-nested-routers
revision: 1
mode: log
language: typescript
description: Avoid explicitly nesting imported routers through host JSX elements and fragments.
scope:
  include: [src/**/*.tsx, src/**/*.jsx]
  exclude: ["**/*.test.*", "**/*.spec.*", "**/fixtures/**"]

let:
  router-modules: [react-router, react-router-dom, react-router/dom,
                   react-router-dom/server, react-router/dom/server, react-router/server]
  router-names:   [BrowserRouter, HashRouter, MemoryRouter, Router, RouterProvider,
                   HistoryRouter, unstable_HistoryRouter, StaticRouter]

utils:
  router-element:
    kind: [jsx_element, jsx_self_closing_element]
    tag:
      kind: identifier
      resolves.import: {module: {in: router-modules}, name: {in: router-names}}

clauses:
  - id: nested-router-jsx
    forbid:
      matches: router-element
      inside:
        matches: router-element
        stopBy:
          any:
            - kind: [arrow_function, function_expression, function_declaration,
                     method_definition, jsx_attribute]
            - kind: jsx_element                    # a composite element
              not: {matches: router-element}
              tag: {not: {kind: identifier, regex: "^[a-z]"}}
    violation:
      code: NESTED_ROUTER_JSX
      message: Remove this inner router and use the outer router for its children.
```

This is a big gain for matcher-only rules. It keeps the shape authors and agents already know from ast-grep. It does not help with relations between rules, recursion across files or aggregates. That is exactly where PR 2539's YAML grew `join`/`where.expr` strings.

### B1. Rerail logic (Datalog), same semantics (30 lines of logic)

This uses the `.rdl` dialect as it exists. It assumes two extensions:

1. `nearest` accepts a derived unary relation, not only a `.kindset`. This is still tape arithmetic: a climb tested against a bitmap.
2. `ref_site(ref, site)` is exposed as an input from the TypeScript scope table. That is the same fact `resolves.binding.site` reads today.

The `=~` operator is regex comparison, which the matcher already has as `regex`.

```prolog
.decl tag(e: node, t: node)
tag($e, $t) :- kind($e, "jsx_element"), field($e, "open_tag", $o), field($o, "name", $t).
tag($e, $t) :- kind($e, "jsx_self_closing_element"), field($e, "name", $t).

.decl router_module(m: sym)
router_module("react-router"). router_module("react-router-dom"). router_module("react-router/dom").
router_module("react-router-dom/server"). router_module("react-router/dom/server"). router_module("react-router/server").
.decl router_export(x: sym)
router_export("BrowserRouter"). router_export("HashRouter"). router_export("MemoryRouter"). router_export("Router").
router_export("RouterProvider"). router_export("HistoryRouter"). router_export("unstable_HistoryRouter"). router_export("StaticRouter").
.kindset import_stmt = "import_statement"

// The local binding created by `import { x as y } from "m"`.
.decl imported(spec: node, m: sym, x: sym)
imported($s, $m, $x) :- kind($s, "import_specifier"), field($s, "name", $n), text($n, $x),
    nearest($s, $i, import_stmt), field($i, "source", $src), string_value($src, $m).

.decl router_el(e: node)
router_el($e) :- tag($e, $t), kind($t, "identifier"), ref_site($t, $s), imported($s, $m, $x),
    router_module($m), router_export($x).

// Walking up from a router, the first of these ends the search.
.kindset barrier = "arrow_function" | "function_expression" | "function_declaration" | "method_definition" | "jsx_attribute"
.decl stop(n: node)
stop($n) :- kind($n, $k), barrier($k).
stop($n) :- router_el($n).
stop($e) :- kind($e, "jsx_element"), !router_el($e), tag($e, $t), !host_tag($t).   // composite; fragments have no tag
.decl host_tag(t: node)
host_tag($t) :- kind($t, "identifier"), text($t, $s), $s =~ "^[a-z]".

.violation NESTED_ROUTER_JSX(at: node) "Remove this inner router and use the outer router for its children."
NESTED_ROUTER_JSX($inner) :- router_el($inner), nearest($inner, $outer, stop), router_el($outer).
```

Plain Datalog is a little longer than A2 for a pure matcher rule. It is shorter for everything A2 cannot express. The fact lists and `tag` belong in a shared library (see C), not in the rule.

### B2. More expressive and more accurate

The rule above misses most real nested routers, because the common bug crosses a component boundary:

| Gap in the current rule | Example that is missed (false negative) or wrongly reported (false positive) |
|---|---|
| Namespace imports | `import * as RR from "react-router-dom"`, then `<RR.BrowserRouter>` (FN) |
| Re-exports and local wrappers | `export { BrowserRouter as AppRouter }`, or `function AppRouter({children}) { return <BrowserRouter>{children}</BrowserRouter> }` (FN) |
| Composites that pass children through | `<BrowserRouter><Layout><MemoryRouter/></Layout></BrowserRouter>`, where `Layout` renders `{children}` (FN) |
| Composites stop the walk even when they *don't* render children | correct today, but only by giving up on every composite |
| A component that renders a router | `<BrowserRouter><App/></BrowserRouter>`, where `App` renders `<RouterProvider>`. This is the usual bug, and it spans files (FN) |
| `Fragment` / `React.Fragment` | treated as a composite, so the walk stops (FN) |
| Render props that render under the router | `<Route element={<MemoryRouter/>}>` (FN: `jsx_attribute` is a barrier) |
| Missing exports | `StaticRouterProvider`, `createRoutesStub` output, react-router v7's `react-router` exports (FN) |

The accurate version resolves tag names to qualified symbols across files. It treats a composite as transparent when its definition is known to render `children`. It reports through component boundaries. When a library component's behaviour is unknown, it gives a `possible` finding instead of silently dropping it. Three new things are needed:

- **`.use` libraries:**
  - `tsx/names`: `sym_of`, which follows imports, aliases, namespace members and re-export chains to a `module#name` symbol.
  - `tsx/jsx`: `tag`, `component`, `render_up`, `children_slot`.

  These are Datalog programs like `calls.rdl`, shared by every rule.
- **`must`/`may` relations** (§12, three-valued results).
- **A YAML header** for the metadata that tools edit (see C).

```prolog
---
id: local:no-nested-routers
revision: 2
mode: log
language: typescript
scope: {include: [src/**/*.{tsx,jsx}], exclude: ["**/*.test.*", "**/*.spec.*", "**/fixtures/**"]}
---
.use tsx/names   // sym_of(n, q): identifier or member chain n denotes exported symbol q, across files and re-exports
.use tsx/jsx     // tag(e, t); component(q, f); children_slot(f, s);
                 // render_up(c, p): p is the element c renders under in this file. Fragments (<>, Fragment,
                 // React.Fragment), {…}, && / ?:, and callbacks in child position (xs.map(x => <X/>)) are
                 // transparent; attributes and other functions end it. Also host_tag, owner(n, f), render_attr(c, e, prop).

.set router_module = "react-router" | "react-router-dom" | "react-router/dom" | "react-router/server"
                   | "react-router-dom/server" | "react-router/dom/server"
.set router_export = "BrowserRouter" | "HashRouter" | "MemoryRouter" | "Router" | "RouterProvider"
                   | "HistoryRouter" | "unstable_HistoryRouter" | "StaticRouter" | "StaticRouterProvider"
.set render_prop   = "react-router#Route.element" | "react-router#Route.errorElement"   // render under the router

// Something that provides router context to what it renders: a router, or a component wrapping children in one.
.decl provider(q: sym)
provider(@qname($m, $x)) :- router_module($m), router_export($x).
provider($q) :- component($q, $f), children_slot($f, $s), outer($s, _).

.decl provider_el(e: node)
provider_el($e) :- tag($e, $t), sym_of($t, $q), provider($q).

// Does element e pass what it renders through to its children? Host elements, providers, and
// composites whose definition renders {children}. A composite with no definition (a package with no summary) may.
.decl passes(e: node)  must, may
must passes($e) :- tag($e, $t), host_tag($t).
must passes($e) :- provider_el($e).
must passes($e) :- tag($e, $t), sym_of($t, $q), component($q, $f), children_slot($f, _).
may  passes($e) :- tag($e, $t), sym_of($t, $q), !component($q, _).

// outer(c, r): c renders under provider element r (one file, tape climb). No negation here: provider → outer →
// provider_el → provider is a positive cycle, so the program stays stratified.
.decl outer(c: node, r: node)  must, may
outer($c, $r) :- render_up($c, $r), provider_el($r).
outer($c, $r) :- render_up($c, $p), passes($p), outer($p, $r).
outer($c, $r) :- render_attr($c, $p, $a), render_prop($a), outer($p, $r).        // <Route element={…}>

// roots(q, r): component q renders provider element r with no provider above it inside q (crosses files).
.decl roots(q: sym, r: node)  must, may
roots($q, $r) :- component($q, $f), provider_el($r), owner($r, $f), !outer($r, _).
roots($q, $r) :- component($q, $f), tag($e, $t), owner($e, $f), !outer($e, _), sym_of($t, $q2), roots($q2, $r).

.violation NESTED_ROUTER_JSX(at: node, outer: node)
    "Remove this inner router; {outer} at line {outer.line} already provides one."
.violation NESTED_ROUTER_VIA_COMPONENT(at: node, inner: node)
    "<{at.name}> renders a router ({inner.path}:{inner.line}) and is used inside another router."

NESTED_ROUTER_JSX($e, $o)            :- provider_el($e), outer($e, $o).
NESTED_ROUTER_VIA_COMPONENT($u, $r)  :- tag($u, $t), !provider_el($u), sym_of($t, $q), roots($q, $r), outer($u, _).
// `must` derivations are findings; `may`-only derivations are findings tagged `possible` (§13: same code, a tag).
```

Notes on B2:

- **Execution.** `outer` is file-local recursion over the tree, so it resolves inside one dispatch (target architecture point 8). `provider` and `roots` recurse across files on interned `module#name` keys. That is the allowed cross-document join, and it runs in waves. `.use` libraries are demand-driven: only `sym_of` facts for JSX tags are installed.
- **Findings stay loose.** Each derivation is a finding key: rule, code, path, line. The message template is presentation.
- **What B2 still can't see:** routers created dynamically (`React.createElement(Router)`, `const R = cond ? A : B`), routers passed as props, and context set up outside the scanned scope. These are reported as nothing, not as `possible`, unless `sym_of` is open for the tag.

### C. Recommendation: Rerail logic files with a YAML header

Use one file type, `*.rr`: a YAML front-matter header, then a Datalog body. Keep `rerail.policy/v1` JSON as the compiled, canonical form that the catalog stores and agents can emit.

- **The header** holds what tools edit:
  - id and revision;
  - mode, which approval changes;
  - scope;
  - ratchet budgets, which tighten mechanically;
  - allowlist artifact paths;
  - language.

  Rerail rewrites the header without touching logic.
- **The body** is the §12 Datalog fragment, plus these additions:

  | Construct | What it adds | Status |
  |---|---|---|
  | `.use lib` | shared libraries such as `python/calls`, `python/imports`, `tsx/names`, `tsx/jsx` | new; this is what removes the 8.3 MB of copies |
  | `.set name = "a" \| "b"` and `$x in {…}` | sugar for fact lists | `.kindset` already exists |
  | `=~` | regex comparison | new |
  | `.violation CODE(at: node, …) "template"` | declares a finding relation, its anchor and its message | new |
  | `must` / `may` | three-valued results | from §12 and §13 |
  | `first { "a" if …; "b" if …; "c" }` | ordered classification | new; desugars to negation chains |
  | `!( a, b )` | negated conjunction | new; desugars to a helper relation |
  | `ast($n, { yaml matcher })` and `` tsx`<$T $$$>` `` atoms | the matcher atom from §12 | the YAML form is A2's syntax |

- **Matcher-only rules can stay pure YAML**, using A2's sugar. Both forms compile to the same AST. A2 `utils` are unary relations, and `inside` + `stopBy` is `nearest` over a relation, so a rule can start in YAML and graduate to `.rr` without changing meaning.

YAML alone doesn't work here: PR 2539 shows that relations, classification and budgets turn YAML into a carrier for expression strings. Plain Datalog alone doesn't either: metadata that tools edit, and long pattern lists, read badly as facts. A bespoke language (QL-style classes, Semgrep-style pattern YAML) adds a third syntax to an architecture that already fixed one AST and two surfaces.

## A data-layer rule in each form

`function_level_imports`, from PR 2539's `lint_function_level_imports.yaml`, abbreviated:

```yaml
classifiers:
  function_import_category:
    input: python_function_import
    cases:
      - when: {expr: "%module IN ('typing', 'typing_extensions', 'collections.abc')", params: {module: module}}
        value: "'typing-only'"
      - when: {expr: "%names_text LIKE '%TYPE_CHECKING%' OR %names_text LIKE '%TypeVar%' OR …"}   # 12 LIKEs
        value: "'typing-only'"
      - when: {expr: "%scope LIKE 'test_%' OR %scope LIKE '%.test_%'"}
        value: "'test-helper'"
      - when: {expr: "%level > 0 OR %module = '' OR … OR (SUBSTR(%module, 1, 1) = 'l' AND SUBSTR(%module, 2, 1) >= '0' AND …)"}   # ~20 SUBSTRs
        value: "'circular-avoidance'"
      - default: "'other'"
# … then relations: (where.expr), rules: (one per code, each re-emitting 7 fields), final_query:, policy.ratchets:
```

The `.rr` versions are in `rule-syntax/`. B1 matches the original Python linter; B2 is the more accurate version.

**The YAML port drifted from the Python.** `scripts/lint_function_level_imports.py` checks exact name membership, `enclosing.startswith("test_")` and `^l\d{3,4}_[a-z][a-z0-9_]*$`. The PR 2539 YAML writes these as SQL `LIKE` strings instead, and SQLite's `LIKE` changes their meaning:
- It is case-insensitive, so `'%Any%'` matches an imported `get_company`.
- `_` is a wildcard, so `'test_%'` matches `testing`.
- It tests `Class.method` instead of the def name.
- It adds `%level > 0`.

Writing rules as expression strings hid these differences. With set literals and regexes in the language, `function-level-imports.b1.rr` states the original exactly.

**What B2 changes.** B2 replaces the name heuristics with evidence:
- **Cycles are proven.** `circular-avoidance` means there is an import-time path from the target module back to the importer. It is a recursive `reaches` relation, demand-driven on (target, importer) pairs.
- **Imports with no cycle get their own code.** Workspace imports with no cycle become `deferred-no-cycle`; B1 counted them as `circular-avoidance`.
- **`typing-only` comes from use.** It means the bound names are read only in annotations (from the scope table), not that they have a typing-ish name.
- **New categories.** `unread`, `optional-dependency` (inside `try`/`except ImportError`) and `heavy-dependency` (`KNOWN_EXPENSIVE_IMPORTS`, which the Python declares and never uses).
- **The exclusion moves to the header.** The `dynamic_import.py` exclusion becomes a header `allow:` row.

Because B2 changes the codes, its budgets have to be re-baselined.

The harder data-layer rules (public-symbol queries, general relations, signed allowlists) need three things: `.use` libraries, cross-file joins on interned keys, and inputs from policy artifacts. The logic body can express all three; A2 cannot.

### Constructs the four `.rr` files use beyond today's `.rdl`

| Construct | Use |
|---|---|
| YAML header: `budget`, `allow`, `possible` | values that tools rewrite; `allow` rows become facts |
| `.use lib` | `tsx/names`, `tsx/jsx`, `python/imports`, `python/names`: Datalog programs like `calls.rdl` |
| `.set`, `in`, `=~`, `!( … )`, `some { … }` | sugar: fact lists, membership, regex, negated conjunction, existential |
| `nearest(n, a, R)` over a derived relation `R` | `inside` + `stopBy` with a rule-defined stop set |
| `within(n, a)` | `inside` with no stop, as tape arithmetic (§12) |
| `first { … }` | ordered classification; desugars to negation chains |
| `.violation CODE(at, …) "template"` | declares a finding relation and its message |
| `must` / `may` relations | three-valued results (§12, §13) |
| `.demand r(bound, bound)` | magic sets (built in Typing-support rounds 8–9) |
| `@qname`, `@cat` | value builtins |

## Open questions

1. Is `*.rr` (header + Datalog) the primary authoring form, with JSON as the compiled form? Or does Datalog stay internal, with authors writing only A2 YAML?
2. Should `tsx/names` cross-file resolution land before the router rule moves to B2? Without it, B2 is B1 plus namespace imports and children transparency.
3. Should `possible` findings default to `log` regardless of the rule's mode?
