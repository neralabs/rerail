# The Intro view. Run by pages.py, which provides w() and block(). Placeholders @@…@@ and {{…}} are filled by
# build.py and by the page from real files and measured stats.

def row(title, paras, blocks):
    w('  <section class="sect snips">\n    <div class="txt">')
    w(f'      <h3>{title}</h3>')
    for p in paras:
        w(f'      {p}' if p.startswith("<") else f'      <p>{p}</p>')
    w('    </div>\n    <div class="code">')
    for b in blocks:
        block(*b)
    w('    </div>\n  </section>\n')

w('''<div class="col">
  <div style="display:grid;gap:14px">
    <h1>Rerail logic: rules as Datalog</h1>
    <p class="lede">A proposal for how Rerail rules are written. It replaces today's <code>rerail.policy/v1</code> JSON with one text format, <code>.rr</code>, and moves name, import and call resolution out of the binary into libraries written in the same format.</p>
    <p class="lede">The format is <a href="https://souffle-lang.github.io/">Soufflé</a> Datalog with four additions for code: syntax patterns, regex captures that bind variables, findings with fixes, and relations that may be unknown.</p>
  </div>
  <h2>Why change</h2>
''')

row("The JSON is hard to read and write", [
    "A rule is a nested matcher tree. Every <em>and</em>, <em>or</em> and helper is another level of objects, and the logic is spread across them.",
    "This is Bandit B102, \"no exec()\", as it ships today, next to the same check in .rr.",
], [
 ("json", open(os.path.join(HERE, "..", "..", "..", "packs", "python", "rules", "bandit-b102-exec-used.json")).read(),
  "Today: packs/python/rules/bandit-b102-exec-used.json"),
 ("rr", '''
#include "python/names.rr"   // sym_of: what an expression refers to
B102(c) :- kind(c, "call"), field(c, "function", f), sym_of(f, "builtins.exec").
B102(c) :- kind(c, "call"), field(c, "function", f),
    kind(f, "parenthesized_expression"),
    child(f, g, 0), sym_of(g, "builtins.exec").     // (exec)(src)
''', ".rr: the same check"),
])

row("It cannot say what the data-layer checks need", [
    "The data layer is a Python codebase of about 9,000 files whose architecture is enforced by custom checks: which layer may import which, sync and async methods in pairs, no import cycles. They follow imports across files and reason over the module graph, so today they are Python scripts, not Rerail rules. The 49 on this page are {{dl_py_lines}} lines of Python. An earlier attempt moved one into YAML rules with embedded SQL, which can only test the text of names.",
    "Example: function-level-imports calls an import \"circular-avoidance\" when the module <em>name</em> looks internal. The .rr version proves the cycle on the import graph.",
], [
 ("src", '''
- when:
    expr: "%level > 0 OR %module = ''
      OR %module IN ('api', 'impl', 'data_layer', '_internal')
      OR %module LIKE 'api.%' OR %module LIKE 'impl.%' OR …
      OR (SUBSTR(%module, 1, 1) = 'l' AND SUBSTR(%module, 2, 1) >= '0'
          AND SUBSTR(%module, 2, 1) <= '9' AND … ))"
  value: "'circular-avoidance'"
''', "Earlier attempt: YAML with embedded SQL (abridged)"),
 ("src", '''
if module.startswith(internal_prefixes) or module in ("api", "impl", "data_layer", "_internal"):
    return "circular-avoidance"
if module == "":  # relative import
    return "circular-avoidance"
if _LAYERED_MODULE_RE.match(first_segment):     # l1000_schema, l9600_graphs, …
    return "circular-avoidance"
''', "Python linter: scripts/lint_function_level_imports.py"),
 ("rr", '''
// Import i, in module a, loads module b, and b already leads back to a through imports that run
// at import time. Moving i to the top of a would close the cycle a → b → … → a.
cycle(i) :- local_import(i), module_of(i, a), loads(i, b),
    b != a, !pkg_above(a, b),
    reaches(b, a).
''', ".rr: function-level-imports.b2.rr"),
])

row("Resolution logic ships in the binary", [
    "What a name refers to, what an import loads and what a call targets are computed in Rust. A JSON rule can only ask through fixed keys such as <code>resolves</code>. Changing how Python scoping or re-exports are handled means changing the binary.",
    "In .rr, the same logic is rules in a library. This is the core of Python name lookup.",
], [
 ("json", '''
"resolves": { "binding": { "form": "import", "site": { "matches": "alias-to-exec" } } }
''', "Today: the rule asks; native/src/code_rule_dsl/{scope,calls,import_target,module_graph} answers (≈28k lines of Rust)"),
 ("rr", '''
// walk: start at the name's own scope and move outward, skipping class bodies, until a scope binds it.
walk(n, x, s) :- nameref(n, x), nearest(n, s, scope).
walk(n, x, p) :- walk(n, x, s), !bound_in(s, x), nearest(s, p, fscope).
look(n, x, s) :- walk(n, x, s), bound_in(s, x).   // the scope that binds it
// sym_of: an imported name gives the module path; a local def its qualified name; else a builtin.
sym_of(n, q) :- qn(n, q0), resolve_export(q0, q).
sym_of(n, q) :- look(n, x, s), defn(s, x, f), qualname(f, q).
sym_of(n, q) :- builtin_ref(n, x), q = @dot("builtins", x).
''', ".rr: lib/python/names.rr ({{lib_py_lines}} lines for all Python libraries)"),
])

w('  <h2>What changes</h2>\n')

row("Settings become directives", [
    "Identity, mode and scope are one line each at the top of the file. Approving a rule or moving a ratchet changes one line.",
], [
 ("json", '''
{
  "format": "rerail.policy/v1",
  "id": "bandit-b102-exec-used",
  "revision": 3,
  "mode": "steer",
  "scope": { "include": ["**/*.py"] },
  "clauses": [ … ]
}
''', "Today"),
 ("rr", '''
.policy "bandit-b102-exec-used" revision 3
.mode steer
.scope include "**/*.py"
''', ".rr"),
])

row("Libraries are Datalog; the binary keeps the basics", [
    "The binary provides facts about the tree and a few operations on it. Everything that interprets the language, such as scopes, imports, calls and JSX, is Datalog that ships as data. A library change is a text change to a <code>.rr</code> file, with no new release.",
], [
 ("rr", '''
.decl kind(n: node, k: symbol)             // a node's grammar kind
.decl field(p: node, f: symbol, c: node)   // c is p's child in field f
.decl within(n: node, a: node)             // n is somewhere below a
.input kind, field, within                 // rows come from the binary
.edit replace(n: loc, text: symbol)        // an edit, for fixes
''', "lib/core.rr: native, in the binary"),
 ("rr", '''
// python/names: `len(xs)` with no local `len` refers to the builtin.
sym_of(n, q) :- builtin_ref(n, x), q = @dot("builtins", x).
''', "lib/python/names.rr: Datalog, shipped as data"),
])

row("Unknown is not false", [
    "Some facts cannot be known, such as what a component from a package renders. Today's rules must guess. A .rr relation can be declared <code>must, may</code>: a finding proven from known facts is definite, and one that needs an unknown fact is reported as <em>possible</em>, with its own mode.",
], [
 ("src", '''
<BrowserRouter>
  <Card><BrowserRouter/></Card>     // Card is workspace code and renders its children
  <Shell><BrowserRouter/></Shell>   // Shell comes from a package with no source
</BrowserRouter>
''', "TSX"),
 ("src", '''
today (JSON)    no findings: the rule stops at any component
.rr             line 2  nested router   definite  → .mode
                line 3  nested router   possible  → .possible
''', "Findings"),
])

row("Fixes are rules", [
    "Today a fix is one text template that replaces the match. In .rr a fix is a set of edits derived from facts, so it can touch several places, and a rule can mark it <code>safe</code> only where its evidence shows the edit changes nothing else.",
], [
 ("json", '''
"fix": "$E"
''', "Today: one template"),
 ("rr", '''
// Hoist a function-level import, but only if it closes no cycle and is a plain statement.
hoist_import(i, "deferred-no-cycle") :- plain_stmt(i), !clash(i) => hoist(i).
''', ".rr"),
 ("diff", '''
  import os
+ from app.storage import read_blob
  def load(path):
-     from app.storage import read_blob
      return read_blob(path)
''', "The edit it makes"),
])

w('''  <h2>Example: no explicitly nested routers</h2>
''')

row("What the rule is for", [
    "In React, a react-router router (<code>&lt;BrowserRouter&gt;</code>, <code>&lt;MemoryRouter&gt;</code>, …) owns the app's location. It keeps the current URL and the navigation history, and hands them to everything rendered below it through React context. <code>&lt;Link&gt;</code>, <code>useNavigate()</code> and <code>&lt;Route&gt;</code> read that context to know where the app is and to move it.",
    "So there must be exactly one router above any piece of the tree. A second router inside the first would start its own history: links below it would change the inner router's location, not the URL bar or the rest of the app. react-router refuses this at render time with “You cannot render a &lt;Router&gt; inside another &lt;Router&gt;. You should never have more than one in your app.”",
    "That error appears only when that screen renders, so tests and review can miss it. It usually comes from code moved between contexts: a page copied from a standalone app, or a component that wraps itself in <code>&lt;MemoryRouter&gt;</code> for a demo or test and is then used inside the app. A static rule catches it before it runs.",
    "This version reports a router inside another router in the same file, with only HTML elements or fragments between them. It stops at anything it cannot see through: a function boundary, an attribute, or a component, whose source it does not read. That makes it precise but incomplete; the two misses below are what the accurate version, on the Data-layer rules tab, adds.",
], [
 ("src", '''
import { BrowserRouter, MemoryRouter } from "react-router-dom";

export function App() {
  return (
    <BrowserRouter>
      <main>
        <MemoryRouter>…</MemoryRouter>   // ← reported: only <main> in between
      </main>
      <Layout>
        <MemoryRouter>…</MemoryRouter>   // missed: the climb stops at <Layout>;
                                         //   the accurate version reads Layout's source
      </Layout>
      <Route element={<MemoryRouter/>} />   // missed: inside an attribute;
                                            //   the accurate version follows `element`
    </BrowserRouter>
  );
}
''', "What it reports"),
])

row("How it works", [
    '''<ol class="steps">
        <li><code>router_el(e)</code>: the tag of element <code>e</code> is a name imported from a react-router package, and the imported name is one of the router exports (<code>BrowserRouter</code>, <code>MemoryRouter</code>, …). It follows the import, so <code>import { BrowserRouter as R }</code> counts.</li>
        <li><code>stop(n)</code>: where the climb up the tree ends. A function or attribute (the relation <code>barrier</code>), any router element, or a component element, meaning a tag that is not a lowercase HTML tag.</li>
        <li><code>NESTED_ROUTER_JSX(inner)</code>: <code>inner</code> is a router, and the nearest <code>stop</code> above it is also a router. <code>&lt;main&gt;</code> and <code>&lt;&gt;…&lt;/&gt;</code> are not stops, so the climb passes through them.</li>
      </ol>''',
    "For the first <code>&lt;MemoryRouter&gt;</code>: climbing up, <code>&lt;main&gt;</code> is not a stop and <code>&lt;BrowserRouter&gt;</code> is, and it is a router, so the rule reports it. For the second, the nearest stop is <code>&lt;Layout&gt;</code>, which is not a router, so it reports nothing.",
], [
 ("rr", '''
// 1. router_el(e): e's tag refers to a router export of a react-router package.
router_el(e) :- tag(e, t), kind(t, "identifier"), ref_site(t, s),
    imported(s, m, x), router_module(m), router_export(x).

// 2. stop(n): the climb ends at functions and attributes, at routers, and at components.
stop(n) :- kind(n, k), barrier(k).
stop(n) :- router_el(n).
stop(e) :- kind(e, "jsx_element"), !router_el(e), tag(e, t),     // not <div>, <main>, …
    count : { kind(t, "identifier"), text(t, s), glob("[a-z]*", s) } = 0.

// 3. The finding: the nearest stop above a router is a router.
NESTED_ROUTER_JSX(inner) :- router_el(inner),
    nearest(inner, outer, stop), router_el(outer).
''', "The three relations, from nested-routers.b1.rr"),
])

w('''  <section style="display:grid;gap:10px">
    <h3>The whole rule, both ways</h3>
    <p style="max-width:75ch">Left: the rule as it is written today. Right: the same meaning in .rr, with comments. A more accurate version, which reads <code>&lt;Layout&gt;</code>'s source across files to see whether it renders its children, is in the Data-layer rules tab.</p>
    <div class="pair">
      <div class="code"><div class="cap">Today: JSON ({{router_json_lines}} lines)</div><div data-json>
@@router_json@@
</div></div>
      <div class="code"><div class="cap">.rr, same meaning ({{router_b1_code}} lines of logic)</div><div data-rr>
@@router_b1@@
</div></div>
    </div>
  </section>

  <section style="display:grid;gap:10px">
    <h2>Where it stands</h2>
    <div class="stats">
      <div class="stat"><b>{{hand_rules}} / {{rules}}</b><span>pack rules written by hand in .rr, each reviewed against its tool</span></div>
      <div class="stat"><b>{{json_mb}} → {{hand_mb}}</b><span>pretty JSON → hand-written .rr, comments excluded</span></div>
      <div class="stat"><b>{{dl_py_lines}} → {{dl_rr_code}}</b><span>data-layer lines: Python → .rr logic</span></div>
    </div>
    <ul class="tight">
      <li>Every pack rule is written by hand on the shared libraries. A second person reviewed each batch against the
        tool it ports (Ruff, Bandit, ESLint and its plugins, Clippy, sqlfluff, PHPStan, …), and the defects found were
        fixed. Each rule's NOTES file lists where it still differs from the tool, and why.</li>
      <li>The rules run in <code>tools/rrlab</code>, a lab harness on Soufflé, and are scored against the tool's own
        findings on its test fixtures.</li>
      <li>The engine reads <code>.rr</code> as its only rule format. With the reader work on its branch
        <code>final-rerail-shape-rr-align</code> (not merged yet), 3,890 of the libraries' 3,906 clauses and 5,220
        of the packs' 5,264 clauses lower. The rest are constructs it still refuses by
        name, such as <code>ordered by</code>, a place used as a value, and recursion whose bound it cannot yet
        prove. <code>docs/rule-syntax/ALIGN-FINAL-SHAPE.md</code> lists every difference and who changes
        what.</li>
      <li>The machine translation of the JSON is kept as a second tab, for comparison.</li>
      <li>49 data-layer checks, function-level-imports and the router rule are also written in their final form,
        beside the Python they replace. Each data-layer check follows its Python linter and is scored with
        <code>rrlab dl</code> against the linter calls recorded from that linter's own test suite.</li>
    </ul>
  </section>

  <section style="display:grid;gap:10px">
    <h2>How to read this page</h2>
    <ul class="tight">
      <li><a href="#syntax">Syntax</a>: Datalog from the start, then what is added to Soufflé, each part of the format, fixes, and possible findings.</li>
      <li><a href="#library">Library</a>: the Datalog libraries and the native core they stand on.</li>
      <li><a href="#packs">Pack rules</a>: every shipped rule, its hand-written .rr beside today's JSON.</li>
      <li><a href="#data-layer">Data-layer rules</a>: the hand-written rules, each beside the JSON port and the Python linter.</li>
      <li>In any .rr code, relation names are links. Blue names come from a Datalog library, orange names are native, and names with a dotted underline are defined in the same file.</li>
    </ul>
  </section>
</div>
''')
