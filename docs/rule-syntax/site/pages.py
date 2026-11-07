"""Writes intro.html and syntax.html, the page's two reading views. Code blocks are HTML-escaped here.

    python3 docs/rule-syntax/site/pages.py      (build.py runs it first)
"""
import html, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))

out = []
def w(s): out.append(s)
LIMIT = 88   # snippet width in characters; the page wraps anything longer with a hanging indent


def reflow(code, kind):
    """Wrap comment lines longer than LIMIT at word boundaries. Code lines and trailing comments are left alone:
    keep those short by hand (the page still wraps them, with a hanging indent)."""
    if kind not in ("rr", "src"):
        return code
    out = []
    for line in code.split("\n"):
        if len(line) <= LIMIT:
            out.append(line)
            continue
        m = re.match(r"^(\s*)(//|#) ?(.*)$", line)
        if m:
            lead = f"{m.group(1)}{m.group(2)} "
            cont = lead + "  "
            cur, first = "", True
            for wd in m.group(3).split(" "):
                pre = lead if first else cont
                if cur and len(pre) + len(cur) + 1 + len(wd) > LIMIT:
                    out.append(pre + cur)
                    cur, first = wd, False
                else:
                    cur = f"{cur} {wd}" if cur else wd
            out.append((lead if first else cont) + cur)
            continue
        out.append(line)
    return "\n".join(out)


def block(kind, code, cap=None):
    c = f'<div class="cap">{cap}</div>' if cap else ""
    code = reflow(code.strip(chr(10)), kind)
    w(f'{c}<div data-{kind}>\n{html.escape(code)}\n</div>')
def sect(title, paras, blocks, extra=""):
    w('  <section class="sect">\n    <div class="txt">')
    w(f'      <h2>{title}</h2>')
    for p in paras:
        w(f'      {p}' if p.startswith("<") else f'      <p>{p}</p>')
    w('    </div>\n    <div class="code">')
    for b in blocks:
        block(*b)
    if extra:
        w(extra)
    w('    </div>\n  </section>\n')

def perf(text):
    """A per-operation performance note, shown at the end of a section's text."""
    return f'<p class="perf"><b>Performance.</b> {text}</p>'

w('''<div class="col">
  <h1>Syntax</h1>
''')

w('''  <section style="display:grid;gap:10px">
    <h2>Datalog in one minute</h2>
    <p style="max-width:75ch">Datalog is a query language over tables. The data is a set of <em>facts</em>, rows in named tables called relations. A <em>rule</em> defines a new relation from existing ones, and its rows can feed further rules. There are no loops or functions: you say which rows a relation has, and the engine works out all of them.</p>
    <p style="max-width:75ch">In Rerail the facts describe the syntax tree of your code. A rule file defines relations over them, and the rows of the relation declared as a <code>.violation</code> are the findings. The four steps below build the rule shown in the next section, which reports calls to Python's <code>exec</code>.</p>
    <p style="max-width:75ch">The language is <a href="https://souffle-lang.github.io/">Soufflé</a> Datalog, with a few additions for code: patterns, regex captures, findings and fixes, and relations that may be unknown. "Soufflé, and what is added" below lists them.</p>
  </section>
''')

def step(title, paras, blocks):
    w('  <section class="sect">\n    <div class="txt">')
    w(f'      <h3>{title}</h3>')
    for p in paras:
        w(f'      <p>{p}</p>')
    w('    </div>\n    <div class="code">')
    for bl in blocks:
        block(*bl)
    w('    </div>\n  </section>\n')

step("1. Facts", [
    "The parser turns each file into facts. Every node of the tree gets an id, written here as <code>#1</code>, <code>#2</code>, …. <code>kind</code> gives a node's type, <code>field</code> links a node to a named child, and <code>text</code> gives a node's source text.",
    "These are ordinary rows: <code>kind(#3, \"call\")</code> is one row of the <code>kind</code> table.",
], [
 ("src", '''
from builtins import exec as run      # the import binds the name `run` to exec
exec(src)                             # #3 a call; its function is #4, the name `exec`
run(src)                              # #5 a call; its function is #6, the name `run`
''', "Python"),
 ("rr", '''
kind(#3, "call").          field(#3, "function", #4).
kind(#4, "identifier").    text(#4, "exec").
kind(#5, "call").          field(#5, "function", #6).
kind(#6, "identifier").    text(#6, "run").
// … and rows for the import, the arguments, and the module itself.
''', "Some of its facts"),
])

step("2. A rule", [
    "<code>head :- body.</code> reads: the head holds wherever every atom of the body holds. Commas mean <em>and</em>. A bare name inside an atom, like <code>c</code> or <code>f</code>, is a variable: the engine tries every value that fits. Strings and numbers are constants.",
    "For <code>c = #3</code>, all three atoms hold, so <code>B102(#3)</code> is derived. For <code>#5</code> the last atom fails: the name is <code>run</code>. This rule misses the alias.",
], [
 ("rr", '''
B102(c) :- kind(c, "call"),              // c is a call node
    field(c, "function", f),             // f is the expression being called
    text(f, "exec").                     // and its source text is exactly "exec"
// Rows derived: B102(#3).   Not derived: B102(#5), because text(#6, "run").
''', "Rule, first try"),
])

step("3. Use a library relation", [
    "<code>sym_of(n, q)</code> says expression <code>n</code> refers to qualified name <code>q</code>. It is not a parser fact: rules in the <code>python/names</code> and <code>python/imports</code> libraries derive it from the facts, following scopes, imports and re-exports. Rules use it like any other relation.",
    "Swap <code>text</code> for <code>sym_of</code> and both calls are found. Click <code>sym_of</code> to see its definition.",
], [
 ("rr", '''
#include "python/names.rr"               // load the library that defines sym_of

// Rows the library derives for the code above:
//   sym_of(#4, "builtins.exec")   `exec` is not bound in this file, so it is the builtin
//   sym_of(#6, "builtins.exec")   `run` is bound by the import to builtins.exec
B102(c) :- kind(c, "call"), field(c, "function", f), sym_of(f, "builtins.exec").
// Rows derived: B102(#3), B102(#5).
''', "Rule, second try"),
])

step("4. Or: several rules, one head", [
    "Write a second rule with the same head; the relation has the rows of both. (Inside one body, <code>( a ; b )</code> is also an <em>or</em>, as in Soufflé.)",
    "Here the second rule covers <code>(exec)(src)</code>, where the called expression is wrapped in parentheses. Negation (<code>!</code>), counting and sets are in \"Bodies\" below.",
], [
 ("rr", '''
B102(c) :- kind(c, "call"), field(c, "function", f), sym_of(f, "builtins.exec").
B102(c) :- kind(c, "call"), field(c, "function", f),
    kind(f, "parenthesized_expression"),    // the call is (…)(src)
    child(f, g, 0),                         // g is what the parentheses wrap
    sym_of(g, "builtins.exec").
''', "Rule, final"),
])

sect("A rule file", [
    "The rule from the steps above, as a complete file: directives first, then the program. <code>.violation</code> declares <code>B102</code> as the findings relation and gives its message.",
    "Relation names in a code view are links. Click one to see its definition. Blue names come from a Datalog library and orange names are native.",
], [
 ("rr", '''
.policy "bandit-b102-exec-used" revision 3   // identity
.description "Avoid exec() calls, including through import aliases."
.language python                 // grammar of files and patterns
.mode steer                      // steer the agent; `block` would stop the edit
.scope include "**/*.py"         // which files the rule reads

#include "python/names.rr"       // sym_of: what a name refers to

// The finding: a relation with one column, the node to report. The string is the message.
.violation B102(at: node)
    "Replace this exec call with code that performs the operation directly."
    suppress nosec "B102"        // `# nosec B102` silences it, as in Bandit
// A call whose function expression refers to builtins.exec. sym_of follows imports, so
// `from builtins import exec as run` still resolves `run` to "builtins.exec".
B102(c) :- kind(c, "call"), field(c, "function", f), sym_of(f, "builtins.exec").
// The same through parentheses: `(exec)(code)`. Child 0 of the parenthesized expression is what it wraps.
B102(c) :- kind(c, "call"), field(c, "function", f),
    kind(f, "parenthesized_expression"),
    child(f, g, 0), sym_of(g, "builtins.exec").
''', "Rule"),
 ("src", '''
exec(code)                          # ← B102 (first rule)
from builtins import exec as run
run(code)                           # ← B102: run refers to builtins.exec
(exec)(code)                        # ← B102 (second rule)

def exec(s): ...
exec(code)                          # not reported: this exec is the local function
''', "What it reports"),
])

w('''  <section class="sect">
    <div class="txt">
      <h2>Soufflé, and what is added</h2>
      <p>Everything a rule body can say comes from Soufflé, so its documentation and its intuitions apply. The additions are for code: they match syntax, carry findings to people and agents, and keep unknown facts apart from false ones.</p>
    </div>
    <div class="tablewrap"><table class="map">
      <tr><th>From Soufflé, unchanged</th><th>Added for Rerail</th></tr>
      <tr><td>Rules <code>h :- b.</code> and facts <code>r("a").</code>; variables are bare names, <code>_</code> is any value</td>
          <td><code>n ~ `exec($ARG)`</code>: n matches a code pattern of the policy's language; each <code>$X</code> in it binds the variable <code>X</code></td></tr>
      <tr><td><code>!</code>, <code>( a ; b )</code>, <code>=</code>, <code>!=</code>, <code>&lt;</code>, arithmetic</td>
          <td>Captures: a named group in <code>match("…(?&lt;X&gt;…)…", s)</code> binds <code>X</code>; <code>match_all</code> gives one row per match</td></tr>
      <tr><td><code>count</code>, <code>min</code>, <code>max</code>, <code>sum : { … }</code></td>
          <td><code>.decl r(…) must, may</code>: a relation that may be unknown, and <code>must</code> / <code>may</code> rules (see "Unknown is not false")</td></tr>
      <tr><td><code>match</code>, <code>contains</code>, <code>cat</code>, <code>strlen</code>, <code>substr</code>, <code>to_number</code>; <code>@functor(…)</code> with <code>.functor</code></td>
          <td><code>.violation</code>, <code>fix</code>, <code>=&gt; edits</code> and <code>.edit</code>: findings and fixes</td></tr>
      <tr><td>Built-in constraints take the pattern first: <code>match(re, s)</code>, <code>contains(sub, s)</code></td>
          <td>Text tests in the same shape: <code>glob("_*cache*", s)</code>, <code>prefix</code>, <code>suffix</code>, <code>word</code>, and <code>i</code> forms that ignore case. Each distinct string is tested once per run, for every rule together</td></tr>
      <tr><td><code>.decl</code>, <code>.type</code>, <code>.input</code>, <code>.output</code>, <code>.pragma</code>, <code>#include</code></td>
          <td>A relation name, or one kind as a string, as the set argument of the tree axes <code>inside</code>, <code>has</code>, <code>nearest</code>, <code>within_stop</code> and <code>precedes_stop</code>: <code>inside(c, f, "function_definition")</code>, <code>nearest(f, s, scope)</code>. They are tape passes, not recursion</td></tr>
      <tr><td>Aggregates <code>count</code>, <code>min</code>, <code>max</code>, <code>sum</code>, over a variable or a term (<code>sum strlen(v) : { … }</code>); none builds a string</td>
          <td><code>s = join x by k sep "," : { … }</code>: the rows' x values in k order, joined. For sorted lists in keys and for the exact bytes a signature covers</td></tr>
      <tr><td>Stratified negation: a relation is complete before anything negates it</td>
          <td><code>.decl r(…) ordered by earlier</code>: r may read itself under <code>!</code> or an aggregate at strictly earlier nodes. For "the latest earlier write that counts" (see "What the data-layer checks need")</td></tr>
      <tr><td>Recursion computes every path; nothing picks one</td>
          <td><code>dfs_first(r, hit, first, edge, target)</code>: the first target a depth-first search from r reaches, edges tried in order. Computed set-based; only cycles need a search (see "The first target of a search")</td></tr>
      <tr><td>Lattice columns: <code>.decl cv(n: node, v: flat&lt;&gt;)</code>, <code>min&lt;&gt;</code>, <code>max&lt;&gt;</code>, or one named by <code>.lattice L&lt;base&gt; { Lub -&gt; @f }</code>; <code>top</code></td>
          <td><code>widen k</code> after the <code>.decl</code>: a value that has changed k times goes to top, so a lattice of unbounded height still ends. Typing keeps constant values and qualified names this way</td></tr>
      <tr><td><code>.pragma "magic-transform" "a, b"</code>: compute a and b only for the arguments rules ask about</td>
          <td>The only pragma read. Typing's call graph uses it, so a rule that asks about three calls does not resolve every call</td></tr>
      <tr><td><code>substr</code>, <code>to_float</code>, <code>%</code>, <code>min</code>/<code>max</code> of two values; <code>&lt;</code> on symbols compares their bytes</td>
          <td>Typing programs: <code>call_target</code>, <code>call_open</code>, <code>type_of</code>, <code>proven_type</code>, … computed before the rules and read through <code>python/calls</code> and <code>python/types</code> (see "Types")</td></tr>
      <tr><td></td>
          <td>Policy directives: <code>.policy</code>, <code>.description</code>, <code>.language</code>, <code>.mode</code>, <code>.scope</code>, <code>.budget</code>, …; <code>#include</code> loads each file once. Names starting <code>rr_</code> are the engine's, and no rule redeclares a name <code>core</code> declares</td></tr>
    </table></div>
  </section>
''')

w('''  <section class="sect">
    <div class="txt">
      <h2>Directives</h2>
      <p>Directives are one per line, so tools can rewrite one without touching the logic. Approving a rule changes <code>.mode</code>, and a ratchet changes <code>.budget</code>.</p>
    </div>
    <div class="tablewrap"><table class="map">
      <tr><th>Directive</th><th>Meaning</th></tr>
      <tr><td><code>.policy "id" revision N</code></td><td>Identity. Starts the file.</td></tr>
      <tr><td><code>.description "…"</code></td><td>One line, shown to people.</td></tr>
      <tr><td><code>.language python</code></td><td>Grammar of the matcher atoms and the default libraries.</td></tr>
      <tr><td><code>.mode log | steer | block</code></td><td>What Rerail does with a finding.</td></tr>
      <tr><td><code>.possible log</code></td><td>Mode for findings that rest on unknown facts (see "Unknown is not false").</td></tr>
      <tr><td><code>.scope include "glob", …</code><br><code>.scope exclude …</code></td><td>Files the rule reads.</td></tr>
      <tr><td><code>.budget REL by code { "a": 12, "total": 40 }</code></td><td>Ratchet: block only past these counts.</td></tr>
      <tr><td><code>.budget * { "total": N }</code></td><td>One budget across every violation the policy declares.</td></tr>
      <tr><td><code>.mode log CODE</code></td><td>After <code>.mode block</code>: one code that only logs.</td></tr>
      <tr><td><code>.output rel</code></td><td>Soufflé. Publish a derived relation as metrics, not findings.</td></tr>
      <tr><td><code>#include "python/names.rr"</code></td><td>Soufflé. Load a library's relations.</td></tr>
      <tr><td><code>.decl r(a: node, b: symbol)</code></td><td>Soufflé. Declare a relation. Types: <code>node</code>, <code>span</code> (a regex capture), <code>loc</code> (either), <code>symbol</code>, <code>number</code>.</td></tr>
      <tr><td><code>scope("module"). scope("lambda").</code></td><td>Soufflé facts. A constant set is a one-column relation; <code>scope(k)</code> tests membership.</td></tr>
      <tr><td><code>.input r(IO=file, filename="…")</code></td><td>Soufflé. Facts from a policy file: allowlists, baselines, registries.</td></tr>
      <tr><td><code>.pragma "magic-transform" "r"</code></td><td>Soufflé. Compute r only for the arguments rules ask about.</td></tr>
      <tr><td><code>.violation CODE(at: node, …) "message {col}"</code></td><td>A finding relation. Continuation lines: <code>suppress</code>, <code>fix</code>, <code>anchor</code>.</td></tr>
      <tr><td><code>CODE(@place("a.json", 1), …) :- …</code></td><td>A finding at a file and line with no node there: an allowlist, a baseline, a Markdown line. Line 0: the whole file.</td></tr>
      <tr><td><code>.violation SEC(at: span, c: symbol) code {c}</code></td><td>The code is a column: each SEC code found in a comment is its own code.</td></tr>
      <tr><td><code>.violation SPTB(…, probe: symbol) tag {probe}</code></td><td>An extra key field, so findings at one place stay apart: one per failing probe.</td></tr>
      <tr><td><code>.severity CODE warning</code></td><td>Severity per code, as the repository's report shows it. Presentation only: it never decides blocking.</td></tr>
      <tr><td><code>.mode log CODE</code></td><td>These codes are reported and never block, for findings that only a ratchet enforces.</td></tr>
      <tr><td><code>.requires "data-layer:a", …</code></td><td>A suite: one finding per member policy that blocks or ends Unknown (budget, timeout).</td></tr>
      <tr><td><code>.gate CODE</code></td><td>When CODE has findings they are the report: a bad signature replaces it. The policy's other files are Unknown, not silently clean.</td></tr>
      <tr><td><code>.on syntax_error report CODE [at line 1]</code></td><td>Also report files that do not parse. Either way such a file gives no facts and its outcome is Unknown.</td></tr>
      <tr><td><code>.on input_error report CODE</code></td><td>Report a policy file that is missing or cannot be read, instead of reading it as empty.</td></tr>
      <tr><td><code>.budget V by code from base [floor] [drop 10%]</code></td><td>Ratchet whose ceilings are the rows of relation base, derived by rules, usually from a verified baseline file. It counts distinct findings. A ratchet on a metric (annotations, tests) is an ordinary rule comparing the metric with its ceiling.</td></tr>
      <tr><td><code>.scope report dirs "api"</code><br><code>.scope text "**/*.md"</code></td><td>Findings only land under these directories / read these files as text only (lines, no tree).</td></tr>
      <tr><td><code>.input r(IO=json, filename="b.json", path="/counts")</code></td><td>Read a repository file: json, yaml, lines (<code>dir=</code> for a directory), file. A missing or malformed file gives an <code>input_error</code> row.</td></tr>
      <tr><td><code>.artifact "test_collection" inputs "**/*.py"</code><br><code>.input r(IO=json, artifact="test_collection")</code></td><td>Facts from a file a CI step wrote by running the repository's code, which Rerail never runs. A change to any input makes the file stale: its rules are then Unknown where they could report.</td></tr>
      <tr><td><code>.edit e(…)</code></td><td>An edit defined in Datalog, for fixes (see "Fixes").</td></tr>
    </table></div>
  </section>
''')

sect("Bodies", [
    "Each rule below is one construct, with what it matches. All of them are Soufflé. A variable inside <code>{ … }</code> stays there unless it is also used outside.",
    perf("<code>kind(n, \"call\")</code> is an index lookup: start a body with it. A text test (<code>glob</code>, <code>match</code>, …) runs once per distinct string in the run, for all rules together, never once per node. A test on <code>text(n, s)</code> of a large node pays for that text once; test a short typed text (<code>name</code>, <code>str</code>) when there is one. <code>!r(n)</code> and <code>count : { … } = 0</code> are anti-joins: r is computed completely first, then looked up."),
], [
 ("rr", '''
// A text test: glob tests the whole text; `*` is any run, `[a-z]` one character of a class.
// On a node it reads the node's text. Matches <div>'s tag `div`, not <Card>'s `Card`.
host_tag(t) :- kind(t, "identifier"), glob("[a-z]*", t).

// match is Soufflé's regex test, for what a glob cannot say: here the shape of a hex literal.
hex_literal(n) :- kind(n, "integer"), match("0[xX][0-9a-fA-F_]+", n).

// A constant set is a relation of facts; membership is an atom.
.decl barrier(kind: symbol)
barrier("function_declaration"). barrier("class_declaration").
stop(n) :- kind(n, k), barrier(k).
// For two or three values, an or inside the body does the same.
stop(n) :- (kind(n, "function_declaration") ; kind(n, "class_declaration")).

// Negation of a relation: a JSX element that is not a router element.
stop(e) :- kind(e, "jsx_element"), !router_el(e).

// "There is none": count : { … } = 0. e and i come from outside; o and b stay inside.
// No open tag of e is a host tag:
stop(e) :- kind(e, "jsx_element"), count : { field(e, "open_tag", o), host(o) } = 0.
// Import i is under no TYPE_CHECKING block:
narrow(i) :- import_stmt(i), count : { within(i, b), type_checking_block(b) } = 0.

// Aggregates: count, min, max, sum. A block with exactly one statement.
single(b) :- kind(b, "block"), k = count : { child(b, _, _) }, k = 1.

// A functor computes a value: @dot("pkg.mod", "f") is "pkg.mod.f" (declared with .functor in core).
// def f inside module pkg.mod gets that qualname.
qualname(f, @dot(m, x)) :- def_name(f, x), nearest(f, s, scope), modname(s, m).

// The first test that holds wins: rank the candidates and keep the lowest (function-level-imports).
candidate(i, 1, "typing-only") :- local_import(i), !value_read(i).
candidate(i, 2, "test-helper") :- local_import(i), within(i, f), test_fn(f).
candidate(i, 9, "other")       :- local_import(i).
FUNCTION_IMPORT(i, c) :- local_import(i), r = min k : { candidate(i, k, _) }, candidate(i, r, c).
''', "Rules"),
 ("src", '''
import typing
def handler():
    from app import models      # local_import. Its candidates: 1 and 9; min picks 1:
    x: models.User = load()     #   models is read only in an annotation → "typing-only"
''', "Python, for the last rule"),
])

sect("Patterns and captures", [
    "Two additions bind variables from code. A pattern atom, <code>n ~ `…`</code>, matches node <code>n</code> against a code pattern in the policy's language, ast-grep's pattern syntax. Each <code>$X</code> in the pattern binds the variable <code>X</code>; by convention, variables bound by patterns are written in capitals and no others are.",
    "A named group in a regex binds a variable too: <code>(?&lt;D&gt;…)</code> binds <code>D</code>. Matched against a node, the capture is a <em>span</em>, a piece of that node's text. A span has <code>text</code> and <code>line</code>, can be a finding's <code>at</code>, and can be the target of an edit, so a finding and its fix can point at four characters of a comment. Matched against a string, the capture is a string.",
    "A group that takes no part in the match gives no row. Inside <code>!</code> or <code>count : { … }</code> a group binds only there, like any variable.",
    perf("A pattern atom is matched only at candidate nodes: the anchor index picks them by kind and by literal text in the pattern, so <code>`exec($A)`</code> visits calls whose callee is <code>exec</code>, not every call. A pattern with no literal and a common kind (<code>`$F($$$)`</code>) visits every call: add the literal, or a cheap atom before it. <code>match</code> compiles to a DFA (no look-around, no back-references), so it costs what a glob costs; captures are extracted after a match, once per distinct string. <code>match_all</code> is linear in the text it scans."),
], [
 ("src", '''
getattr(user, "email")       # ← B009: fix writes user.email
x = 1  # todo: retry         # ← TD006 at "todo": fix writes TODO
''', "Python"),
 ("rr", '''
// The pattern binds FUNC, OBJ and NAME to the call's parts; the rest of the body tests them.
B009(n) :- n ~ `$FUNC($OBJ, $NAME)`, getattr_ref(FUNC), plain_arg(OBJ), attr_name(NAME, _).
// In the fix template, $OBJ is OBJ's source text and $a is a's value.
to_attr(n) :- n ~ `$FUNC($OBJ, $NAME)`, attr_name(NAME, a) => replace(n, `$OBJ.$a`).

// The group D is a span of the comment: the finding is at those four characters.
.violation TD006(at: span) "Write `{at.text}` as `TODO`."
    fix safe upper "Write TODO"
TD006(D) :- kind(c, "comment"), match("#\\s*(?<D>(?i:todo))\\b.*", c), text(D, t), t != "TODO".
upper(D) => replace(D, `TODO`).

// Matched against a string, a capture is a string: "use_foo" gives x = "foo".
hook_name(f, x) :- def_name(f, n), match("use_(?<x>.*)", n).
''', "Rules"),
])

w('''  <section class="sect">
    <div class="txt">
      <h2>From JSON to .rr</h2>
      <p>Every JSON construct has a fixed lowering. The tree axes become native tree relations: <code>within</code> is "below", and <code>within_stop</code> is "below, with no stop node in between". Utilities become relations. ast-grep shares metavariables between a rule and its utilities; a Datalog variable belongs to one rule, so a utility that reads or binds a metavariable its caller uses gets it as an extra column. Anything without a lowering stays as a matcher atom, <code>ast(n, {…})</code>, and runs as it does today.</p>
      <p class="perf"><b>Performance.</b> The tree axes with a set (<code>inside</code>, <code>has</code>, <code>nearest</code>, <code>within_stop</code>, <code>precedes_stop</code>) are one pass over the document's tape per axis and set, not recursion. Plain <code>within(n, a)</code> relates a node to every ancestor, so its rows grow with nesting depth: prefer <code>inside(n, a, "kind")</code> or <code>nearest</code> when the ancestor's kind is known.</p>
    </div>
    <div class="tablewrap"><table class="map">
      <tr><th>JSON</th><th>.rr</th></tr>
      <tr><td><code>"kind": "call"</code></td><td><code>kind(n, "call")</code></td></tr>
      <tr><td><code>"regex": "^exec$"</code>, <code>"_async$"</code>, <code>"^(?:api/)?impl/"</code></td><td><code>glob("exec", n)</code>, <code>suffix("_async", n)</code>, <code>glob("{api/,}impl/**", n)</code>: a glob test wherever it is exactly the regex (1,949 of 2,123 distinct patterns)</td></tr>
      <tr><td><code>"regex": "0[xbo][0-9A-Fa-f_]*_(?:8|16)"</code></td><td><code>match("0[xbo][0-9A-Fa-f_]*_(?:8|16)", n)</code>: whole-text, so <code>^</code> and <code>$</code> go and <code>.*</code> marks open ends</td></tr>
      <tr><td><code>"regex": "(?&lt;D&gt;todo)"</code></td><td><code>match(".*(?&lt;D&gt;todo).*", n)</code>, which binds the span <code>D</code></td></tr>
      <tr><td><code>"pattern": "$F($$$)"</code></td><td><code>n ~ `$F($$$)`</code>, which binds <code>F</code></td></tr>
      <tr><td><code>"has": {"field": "f", …}</code></td><td><code>field(n, "f", c), …</code></td></tr>
      <tr><td><code>"has": {…}</code></td><td><code>child(n, c, _), …</code></td></tr>
      <tr><td><code>"has": {"stopBy": "end", "kind": "k", …}</code></td><td><code>has(n, c, "k"), …</code></td></tr>
      <tr><td><code>"inside": {"stopBy": "end", "kind": "k", …}</code></td><td><code>inside(n, p, "k"), …</code></td></tr>
      <tr><td><code>"has"</code> / <code>"inside"</code> <code>{"stopBy": "end", …}</code>, no kind</td><td><code>within(c, n), …</code> / <code>within(n, p), …</code></td></tr>
      <tr><td><code>"inside": {"stopBy": R, …}</code></td><td><code>within_stop(n, p, R), …</code></td></tr>
      <tr><td><code>"precedes"</code> / <code>"follows"</code></td><td><code>next_sibling</code>, <code>precedes</code>, <code>precedes_stop</code></td></tr>
      <tr><td><code>"all": […]</code></td><td>Commas</td></tr>
      <tr><td><code>"any": […]</code></td><td>One rule per alternative; plain kinds fold into <code>(kind(n, "a") ; kind(n, "b"))</code>, or a relation of facts for more than four</td></tr>
      <tr><td><code>"not": …</code></td><td><code>!r(n)</code>, or <code>count : { … } = 0</code></td></tr>
      <tr><td><code>"utils": {"u": …}</code>, <code>"matches": "u"</code></td><td><code>.decl u(n: node)</code> and its rules; <code>u(n)</code>. Shared metavariables become extra columns: <code>u(n, NAME)</code></td></tr>
      <tr><td><code>"constraints": {"F": …}</code></td><td>Literals on <code>F</code></td></tr>
      <tr><td><code>"value": {"eq": "x"}</code></td><td><code>value(n, "x")</code></td></tr>
      <tr><td><code>"nthChild": 2</code></td><td><code>child(_, n, 1)</code></td></tr>
      <tr><td><code>"count": {…, "min": 1}</code></td><td><code>k = count : { … }, k &gt;= 1</code></td></tr>
      <tr><td><code>"resolves": {"import": […]}</code></td><td><code>sym_of(n, "a.b")</code>, or <code>( … ; … )</code> for several</td></tr>
      <tr><td><code>"resolves": {"binding": {"site": …}}</code></td><td><code>ref_site(n, s), …</code></td></tr>
      <tr><td><code>"message": "… $NAME …"</code>, <code>"each"</code>, <code>"anchor": "capture.C"</code></td><td>Columns of the finding: <code>.violation V(at: node, name: loc) "… {name} …"</code>. One finding per distinct row, which is what <code>each</code> asked for</td></tr>
      <tr><td><code>"fix": "$E"</code></td><td><code>… =&gt; replace(n, `$E`)</code> on each rule of the clause</td></tr>
      <tr><td><code>"fix": {"template": "", "range": "capture.X"}</code></td><td><code>… =&gt; replace(X, ``)</code></td></tr>
      <tr><td><code>"$option": "name"</code></td><td><code>option("name", v)</code></td></tr>
      <tr><td><code>"regex": {"$option": "o", "cases": {…}}</code>, and option-valued operands</td><td>One constant per configured value: <code>(option("o", "never"), match("…", n) ; !option("o", "never"), match("…", n))</code>, so capture groups still bind</td></tr>
      <tr><td>An <code>any</code> alternative that does not bind <code>$X</code></td><td><code>unset(X)</code>: core's one empty value, which ast-grep gives a metavariable its match left unbound. Its text is <code>""</code></td></tr>
      <tr><td><code>"transform": {"T": {"replace": {"source": "$S", …}}}</code></td><td><code>text(S, t), T = @replace_all(t, "re", "by")</code>; likewise <code>@convert</code>, <code>@slice</code>, <code>@to_precision</code>, and <code>line(S, T)</code>, <code>list_len(S, T)</code> for <code>line</code> and <code>count</code></td></tr>
      <tr><td><code>mode</code>, <code>scope</code>, <code>violation</code></td><td>Directives</td></tr>
    </table></div>
  </section>
''')

FIX_TABLE = '''<div class="tablewrap"><table class="map">
      <tr><th>Form</th><th>Meaning</th></tr>
      <tr><td><code>fix NAME "title"</code></td><td>A suggestion, shown with the finding. The agent or a person applies it.</td></tr>
      <tr><td><code>fix safe NAME "title"</code></td><td>Rerail may apply it without review. Never offered for a <code>possible</code> finding.</td></tr>
      <tr><td>Several <code>fix</code> lines</td><td>Alternatives; the reader picks one.</td></tr>
      <tr><td>Several rules for one fix</td><td>Their edits combine into one fix.</td></tr>
      <tr><td><code>V(…) :- body =&gt; edits.</code></td><td>An unnamed suggestion for this match, using its variables.</td></tr>
      <tr><td><code>`…`</code> in an edit</td><td>A template in the rule's language. <code>$x</code> is variable x: a node's or span's source text, or a value; <code>$$$X</code> is a list a pattern bound. Reindented where it lands. Fence with more backticks to contain one. Templates are the one place outside patterns where <code>$</code> appears.</td></tr>
      <tr><td>Applying</td><td>All edits of a fix, across files, or none. Dropped if two edits overlap or the result does not parse. Rule tests check that the finding is gone afterwards.</td></tr>
    </table></div>'''

sect("Fixes", [
    "A fix is a set of edits, and rules derive edits like any other rows. A rule that ends in <code>=&gt; e1, e2</code> adds those edits to the fix named in its head. A fix relation has its violation's columns; Rerail fills them from each finding, so a fix's rules run once per finding.",
    "The binary applies four edits: <code>replace</code>, <code>delete</code>, <code>insert_before</code>, <code>insert_after</code>. Edits declared with <code>.edit</code>, such as <code>hoist</code> and <code>remove_stmt</code>, are Datalog in the libraries and expand to those four, so they change without a new binary.",
    perf("Fix rules are asked for findings only, one question per finding, so a costly safety check in a fix body costs nothing on clean files."),
    "The example is from function-level-imports. One file has two imports inside functions:",
    '''<ol class="steps">
        <li><code>FUNCTION_IMPORT</code> finds both and gives each a code: <code>"deferred-no-cycle"</code> and <code>"unread"</code>.</li>
        <li>For the first, Rerail asks <code>hoist_import(&lt;import&gt;, "deferred-no-cycle")</code>. Its body checks that the move is harmless and derives the edit <code>hoist(&lt;import&gt;)</code>.</li>
        <li><code>hoist</code> expands: <code>remove_stmt</code> becomes <code>delete</code> (the function has another statement), and <code>add_top</code> becomes <code>insert_after(import os, …)</code>.</li>
        <li>The native applier turns those into byte edits, reindents the moved line, applies both and reparses. The fix is <code>safe</code>, so Rerail may apply it on its own.</li>
        <li>The second finding gets <code>remove_import</code>. It is a suggestion: the module may be imported for a side effect, so the agent or a person decides.</li>
      </ol>''',
], [
 ("src", '''
import os

def load(path):
    from app.storage import read_blob   # ← "deferred-no-cycle": app.storage
    return read_blob(path)              #   does not import this module

def debug():
    import app.plugins.audit            # ← "unread": nothing reads `app`
''', "Python before"),
 ("rr", '''
.violation FUNCTION_IMPORT(at: node, code: symbol) "{at.text}"
    // safe: Rerail may apply it unreviewed
    fix safe hoist_import "Move `{at.text}` to the top of the module"
    // a suggestion (the default)
    fix remove_import "Delete `{at.text}`: nothing reads what it binds"

// Fix 1. The head's columns are the finding's: i is the import, "deferred-no-cycle" its code, so
// this rule applies only to findings with that code. The body is the safety argument:
//   plain_stmt(i)  the import is a direct statement of the function body, not inside if/try/with,
//                  so moving it cannot skip an exception handler or a condition;
//   !clash(i)      no module-level name is already bound to something else under the same name.
// `=> hoist(i)` is the edit: move this statement to the top of the module.
hoist_import(i, "deferred-no-cycle") :- plain_stmt(i), !clash(i) => hoist(i).

// Fix 2. No body: every "unread" finding gets it. remove_stmt deletes the statement, or writes
// `pass` when it was the only statement, so the function stays valid Python.
remove_import(i, "unread") => remove_stmt(i).
''', "Rule"),
 ("rr", '''
// hoist (python/imports). `$i` in the template is the statement's own text.
.edit hoist(i: node)
// Some name it imports is not imported at the top yet: move it.
hoist(i) :- docof(i, d),
    count : { import_alias(i, _, m, x, l), !top_has(d, m, x, l) } > 0
    => remove_stmt(i), add_top(d, `$i`).
// Already imported at the top: just delete the copy.
hoist(i) :- docof(i, d),
    count : { import_alias(i, _, m, x, l), !top_has(d, m, x, l) } = 0
    => remove_stmt(i).
''', "Library edit, one level down"),
 ("diff", '''
  import os
+ from app.storage import read_blob       # add_top: after the last top-level import

  def load(path):
-     from app.storage import read_blob   # delete: `return …` remains
      return read_blob(path)
''', "After hoist_import (safe)"),
 ("diff", '''
  def debug():
-     import app.plugins.audit
+     pass                                # replace: the body would be empty
''', "After remove_import (suggestion, if accepted)"),
 ("rr", '''
// Inline form: the fix of this one match, using its variables. Today's JSON fix templates become this.
// The pattern binds E to the expression before the doubled `!`; the edit replaces the whole match.
NO_DOUBLE_BANG(n) :- n ~ `$E!!` => replace(n, `$E!`).
''', "Inline fix"),
 ("diff", '''
- const name = user!!.name;
+ const name = user!.name;     // E is `user`; `user!` replaces `user!!`
''', "TypeScript, before and after"),
], FIX_TABLE)

sect("Native or Datalog", [
    "The binary provides the parse tape, tree arithmetic, value builtins, file readers and the edit applier. Name resolution, imports, JSX structure and most edits are Datalog libraries over those. Call targets and types come from typing programs, also <code>.rr</code>, that the engine runs before the rules. A library or program update ships without a new binary.",
    '''<ul class="tight">
        <li><code>core</code>: native. Tape relations, <code>within</code>, <code>nearest</code>, functors, config readers, the four edits.</li>
        <li><code>python/names</code>, <code>python/imports</code>: adapted from an earlier Datalog prototype of Python call resolution.</li>
        <li><code>python/calls</code>, <code>python/types</code>: on the engine, joins over the Python typing program's answers. In the lab harness, <code>python/calls</code> is still the prototype's resolver, because the harness does not run typing programs.</li>
        <li><code>tsx/names</code>, <code>tsx/jsx</code>: drafted for this page.</li>
      </ul>''',
    perf("A library relation is computed once per run and shared by every policy that reads it, so a second policy over <code>python/names</code> adds only its own rules. Name resolution and imports span the workspace, so they cost the most: reach them through a selective atom (<code>sym_of(t, q)</code> after <code>kind(t, \"identifier\")</code> and a name test), not as a whole relation."),
], [
 ("rr", '''
#include "python/names.rr"     // scope_node, binding, ref_site, sym_of, annotation, catches, …
#include "python/imports.rr"   // import_alias, loads, reaches, exports, hoist, …
#include "python/calls.rr"     // call_target, call_open, reach, call_arg, awaited, …
#include "python/types.rr"     // type_of, type_open, proven_type, possible_type, possible_open

// core (native): kind, field, child, text, value, within, nearest, line, doc, file, tsconfig_path,
//                replace, delete, insert_before, insert_after
''', "What a Python rule can use"),
])

sect("Unknown is not false", [
    "Some facts cannot be known. Below, <code>Shell</code> comes from a package Rerail has no source for, so whether it renders its children is unknown. A two-valued rule must pick: treat unknown as false and miss real bugs, or as true and report things that may be fine.",
    "A relation declared <code>must, may</code> is computed twice. <code>must</code> holds what is proven; <code>may</code> holds what is proven or unknown. A rule prefixed <code>must</code> or <code>may</code> adds rows to that side only. A rule with no prefix adds to both.",
    "Negation swaps sides. <code>must stuck(e) :- …, !passes(e)</code> reads the may side: the element is surely stuck only if it surely does not pass. So a definite finding never rests on an unknown.",
    "A finding that holds only on the may side keeps its code, is tagged <code>possible</code>, and uses the <code>.possible</code> mode, here <code>log</code> instead of <code>block</code>.",
    perf("A <code>must, may</code> relation is computed twice, and everything that negates it reads both sides. Declare it only where facts can really be missing."),
], [
 ("src", '''
import { BrowserRouter } from "react-router-dom";
import { Card } from "./Card";      // ours: returns <div>{children}</div>
import { Shell } from "@acme/ui";   // a package: no source, so unknown

<BrowserRouter>
  <Card><BrowserRouter/></Card>     // ← definite: Card renders its children
  <Shell><BrowserRouter/></Shell>   // ← possible: only if Shell does
</BrowserRouter>
''', "TSX being checked"),
 ("rr", '''
// passes(e): whatever e is given as children ends up rendered inside it, so the outer router's
// context reaches them. The nested-router rule climbs from an inner router through passing elements.
.decl passes(e: node)  must, may

// <div>, <main>: host elements always render their children. Proven, so on the must side.
must passes(e) :- tag(e, t), host_tag(t).
// <Card>: a workspace component whose source has {children} in what it returns. Also proven.
must passes(e) :- tag(e, t), sym_of(t, q), component(q, f), children_slot(f, _).
// <Shell>: no source (no component row), so it might pass children. Only the may side gets this row.
may  passes(e) :- tag(e, t), sym_of(t, q), !component(q, _).

// For the code above:
//   <Card>   must passes, may passes   → the inner router in Card is reported with .mode
//   <Shell>  may passes only           → the inner router in Shell is reported as possible, .possible mode
''', "Rule (from the accurate nested-routers rule)"),
])

TYPES_TABLE = """<div class="tablewrap"><table class="map">
      <tr><th>Compared with</th><th>Same answer</th><th>Departs</th><th>Wrong</th></tr>
      <tr><td>Python calls, against what CPython calls when the fixtures run</td><td>113</td><td>9</td><td>0</td></tr>
      <tr><td>Python receiver types, against the classes seen at run time (a small fixture written for this)</td><td>44 of 44</td><td>0</td><td>0</td></tr>
      <tr><td>Rust calls, against rust-analyzer</td><td>37</td><td>7</td><td>0</td></tr>
      <tr><td>Python calls on django (907 files), against Jedi, where both decide</td><td>20,205 (99.2%)</td><td>159</td><td>not judged</td></tr>
    </table></div>"""

sect("Types", [
    "Many checks are about what a value is, not how it is spelled: a write on a read-only connection, a blocking call reached from async code, a method called on the wrong class. A text test on the receiver's name finds some of these and misses every alias. Types make them checks about values.",
    "Rerail does not run a type checker. A typing program, itself written in <code>.rr</code>, infers what each expression may hold from what the code assigns, returns, constructs and inherits, and keeps every gap as unknown. The engine runs it over the repository before the rules, file by file and then in a few cross-file rounds, and only when a rule reads one of its answers.",
    "Rules read the answers as ordinary relations, from two libraries. They are three-valued, like <code>must, may</code>: <code>proven_type(n, t)</code> means <code>isinstance(n, t)</code> is surely true; <code>possible_type(n, t)</code> or <code>possible_open(n)</code> means it may be. A call has its <code>call_target</code> rows, and <code>call_open</code> when it may run something else as well. A missing row is never read as false while the open relation holds.",
    """<ul class="tight">
        <li><code>python/calls</code>: <code>call_target(c, f)</code>, <code>call_open(c)</code>, and the call graph built on them: <code>calls</code>, <code>calls_open</code>, <code>reach</code>.</li>
        <li><code>python/types</code>: <code>type_of(n, t)</code>, <code>type_open(n)</code>, <code>proven_type</code>, <code>possible_type</code>, <code>possible_open</code>. A class is its qualified name (<code>"app.db.ReadOnlyConnection"</code>); a builtin is its bare name (<code>"str"</code>, <code>"None"</code>).</li>
      </ul>""",
    "The program answers at the places whose value it models: callees and receivers, and values that are assigned, returned, passed, or used as decorators and bases. It does not read annotations yet, so a parameter's type is open; generics carry no arguments (<code>list</code>, not <code>list[int]</code>). Calls of libraries with no source are typed from declared return types, in stub files and dependency summaries. Python is first; Rust calls are resolved, and TypeScript, PHP and narrowing are next.",
    "The table scores the answers against what the code really does. A departure is an answer that is not the reference's, each pinned with its reason (in Rust, trait dispatch and iterator chains); wrong counts definite answers that were wrong. On django, 7,286 calls that Jedi decides are still open: that is the remaining work.",
    perf("The program runs once per revision of each file and is shared by every rule that reads it; demand limits the call graph to the calls rules ask about. Cross-file rounds are bounded, and a round cut short marks the calls it left without a target as open, so an unfinished answer reads as unknown, never as <em>calls nothing</em>."),
], [
 ("src", """
# app/db.py
class Connection: ...
class ReadOnlyConnection(Connection): ...

# app/report.py
from app.db import ReadOnlyConnection

def nightly(sql):
    conn = ReadOnlyConnection()
    conn.execute_write(sql)       # ← proven: conn is a ReadOnlyConnection

def handler(conn, sql):
    conn.execute_write(sql)       # ← possible: conn's class is unknown
""", "Python being checked"),
 ("rr", """
#include "python/types.rr"    // proven_type, possible_type, possible_open
#include "python/names.rr"    // attr_of

.violation RO001(at: node) "execute_write on a read-only connection"
.violation RO002(at: node) "execute_write on a value that may be a read-only connection"
.mode log RO002               // unknown receivers are logged, not blocked

// write(c, r): c is a call `r.execute_write(…)`, with receiver r.
.decl write(c: node, r: node)
write(c, r) :- kind(c, "call"), field(c, "function", f), attr_of(f, r, "execute_write").

// Every class r may hold is ReadOnlyConnection or a subclass: a definite finding.
RO001(c) :- write(c, r), proven_type(r, "app.db.ReadOnlyConnection").
// Not proven, but it may be one, or its class is unknown.
RO002(c) :- write(c, r), !proven_type(r, "app.db.ReadOnlyConnection"),
    ( possible_type(r, "app.db.ReadOnlyConnection") ; possible_open(r) ).
""", "Rule (an illustration: no shipped rule reads types yet)"),
], TYPES_TABLE)

sect("What the data-layer checks need", [
    "The data-layer repository has 142 lint checks, written in Python. Each was read against the language, to find what it needs beyond what is here. Most needs are facts the Python checks take from CPython (its <code>ast</code>, its string decoder, its visit order) and repository files they read (allowlists, signed baselines, Markdown). Those became native facts and library relations, not syntax.",
    "Three needs became language: <code>join</code>, because nothing else builds the exact text a signature covers; <code>ordered by earlier</code>, for state where only some writes count; and <code>dfs_first</code>, below. Most state needs no new form: when every assignment resets a name, the latest write is plain syntax, and the recursion is positive. Signatures are checked in Datalog, so a repository's own signing layout needs no new reader.",
    "Checks that run repository code (pytest collection, import-time registries) stay outside: a CI step writes the facts to a file and the policy reads it. Rerail never runs the code it checks.",
    perf("<code>join</code> sorts each group, so it is meant for short lists (an allowlist, a key), not whole files. An <code>ordered by earlier</code> relation is evaluated one document at a time in visit order, which is sequential: use it only when which write counts depends on the relation itself; a plain <code>max</code> over <code>earlier</code> is set-based. <code>@unparse</code>, <code>@ast_dump</code> and <code>str_part</code> come from an isolated CPython worker, one batch per wave: put a cheap filter before them so only the nodes that need them are sent."),
], [
 ("rr", '''
// lib/data-layer/signing.rr: the signed bytes of an allowlist, rebuilt and verified.
// The repository signs json.dumps({"allowlist": sorted(entries), "linter": …, "version": 1}).
allow_message(l, msg) :- allow_sig(l, linter, _),
    body = join q by e sep "," : { allow_entry(l, e), q = @json_str(e) },
    msg = cat("{\\"allowlist\\":[", body, "],\\"linter\\":", @json_str(linter), ",\\"version\\":1}").
allow_status(l, "unsigned") :- allow_sig(l, _, s), unsigned_sig(s).
allow_status(l, v) :- allow_sig(l, _, s), !unsigned_sig(s), allow_message(l, m),
    ok = count : { nera_key(k), @ed25519_ok(k, m, s) = 1 },
    ( ok > 0, v = "valid" ; ok = 0, v = "invalid" ).
''', "Signatures in Datalog (checked against the repository's real signatures)"),
 ("rr", '''
// data-layer/file-length.rr: a bad signature replaces every other finding.
invalid_allowlist_signature(@place(".file-length-allowlist.json", 1)) :- allowlisted(_, _),
    !allow_status("file_length", "valid").
.gate invalid_allowlist_signature
''', "A gate at a place"),
 ("rr", '''
// A name's code is the code of its latest earlier assignment whose value had one: `x = a.field`
// then `x = b` leaves x at EOP002. Which write counts depends on src_code itself, under max.
.decl src_code(v: node, c: symbol) ordered by earlier
src_code(n, c) :- ast_name(n, x), expr_ctx(n, "load"), frame(n, f),
    k = max i : { write(w2, f, x, v2), earlier(v2, n), src_code(v2, _), visit_index(w2, i) },
    visit_index(w, k), write(w, f, x, v), src_code(v, c).
''', "ordered by earlier (data-layer/sketches/owner-code.rr)"),
])


sect("The first target of a search", [
    "Recursion finds every path. Some checks report the path a depth-first search finds first: \"the function reaches an eager <code>resolve()</code>, reported at the first call that leads there\". Which call is first depends on order, so a closure cannot say it, and a closure reports every call that leads there.",
    "<p><code>dfs_first(r, hit, first, edge, target)</code> answers it. <code>edge(src, dst, ord)</code> is any relation of three columns: from each node the search tries edges in increasing <code>ord</code>, enters each node once, and skips nodes on its current path. <code>hit</code> is the first node of <code>target</code> an edge reaches, and <code>first</code> is the <code>ord</code> of r's edge that starts the path, so <code>visit_index(c, first)</code> gives the call to report. r is never its own hit; write <code>target(r)</code> beside it for that. No row means no target is reachable.</p>",
    "<p><code>dfs_first_memo(…, roots)</code> is the variant some scripts have: roots are searched in the order of <code>roots</code>' second column (numbers, or symbols compared as text, as Soufflé's <code>&lt;</code> does), one memo is shared, and a node finished while a cycle was open counts as reaching nothing. It reports fewer findings on cycles, and which ones depends on the names: over the data layer's whole repository it lost 34 of 1,864 functions, all in one group of renderers that call each other. Use it only where findings must match such a script.</p>",
    "The answer is only as exact as <code>target</code>. LEG-083 counts any method named <code>resolve</code>. At the data layer's October commit, every one of its 18 findings in its own files is a pathlib path or a lookup with arguments. Saying what a lazy terminal is (no arguments, and a receiver that is not a path) removes them all, and keeps all 40 real findings at the August commit.",
    "Both take relations as arguments, like <code>nearest</code>, so those relations must be complete first: the checker treats them as negated. A known answer or a node to avoid is a rewrite of the arguments, not an option: drop a node's edges to avoid it; replace them with one edge to its target to fix its answer.",
    perf("Set passes, then a search only where cycles are. The engine computes which nodes can reach a target, each node's first edge into one, and where each chain of first edges ends; a root whose chain stays out of every cycle is answered from that, whatever the order of roots. Only chains that enter a cycle are searched, inside the cycle, and without the shared memo each entry point is searched once. A cycle through most of the graph keeps a search that large: depth-first order on general graphs is inherently sequential."),
], [
 ("rr", '''
// The call graph: f's calls in visit order, to the eager call itself
// or to the def it resolves to.
leg_edge(f, c, i) :- eager_resolve(c), py_frame(c, f), visit_index(c, i).
leg_edge(f, g, i) :- name_call_target(c, g), py_frame(c, f),
    visit_index(c, i).
// The target decides what counts. A lazy terminal takes no
// arguments, and a pathlib path's resolve() is not one.
eager_call(c) :- eager_resolve(c), callee(c, m), attr_of(m, o, _),
    !call_arg(c, _, _, _), !path_value(o).

// One finding per function that reaches an eager call, at the call
// that starts the first path.
LEG_083(c) :- leg_root(f, _), dfs_first(f, _, i, leg_edge, eager_call),
    py_frame(c, f), visit_index(c, i).
''', "Rule (data-layer/sketches/transitive-eager.rr)"),
 ("src", '''
def load(x):
    check(x)          # check reaches no eager call
    return fetch(x)   # fetch -> resolve()   ← LEG_083 reported here

def fetch(x):
    return plan(x).resolve()

def home(p: Path):
    return p.resolve()  # a path, not a lazy value: no finding
''', "Python being checked"),
 ("rr", '''
// Avoid node u: drop its edges and the edges into it (an edge into
// an avoided target still counts as a hit, so keep those).
// Fix node v's answer to target t at location l: one edge, v -> t.
edge2(u, d, o) :- leg_edge(u, d, o), !avoid(u),
    (!avoid(d) ; eager_call(d)), !known(u, _, _).
edge2(v, t, l) :- known(v, t, l).
''', "Known answers and avoided nodes, as a rewrite of the edges"),
])

sect("Code inside code (proposed)", [
    "Code often holds other code: SQL in a Python string, expressions in a template, a type annotation written as a string. Today a rule reaches such text through <code>parsed(s, grammar, root)</code>, which the binary provides for a few grammars.",
    "The proposal makes each one a guest document, declared by a library rule rather than built in. A guest has its own grammar, nodes, kinds and lines, so every rule of that language runs on it unchanged; relations tie its nodes back to the host. A policy that should see only real files says <code>.scope embedded false</code>.",
    "A parsed tape would also become a value inside rules, so a fix can splice new code into a tree and print it. None of this is built yet.",
], [
 ("rr", """
// A library declares where guests are: here, the SQL a Python call passes to execute().
embed(at, "sql", "program") :- kind(c, "call"), field(c, "function", f), attr_of(f, _, "execute"),
    posarg(c, 0, at), str(at, _).

// Rules read guests like documents:
//   guest(g, host, at, grammar)  document g is the text at `at` in document host, parsed as grammar
//   doc_role(d, r)               "primary" (a file), "program" or "fragment" (a guest)
//   host_node(n, h)              h is the innermost host node spanning guest node n
//   root(d, n)                   d's root; for a fragment, the expression inside its wrapper
// A SQL rule reports inside the string; host_node puts the finding on the Python line.
""", "The proposed relations"),
])

w('''  <section class="sect">
    <div class="txt">
      <h2>What the engine runs today</h2>
      <p>The engine reads <code>.rr</code> as its only rule format. Of the clauses on this page, it lowers 4,256 of the libraries' 4,277, 5,202 of the packs' 5,264 and 2,403 of the data-layer checks' 2,418. The rest are constructs it refuses by name, never runs wrongly.</p>
      <p>Most of the language runs: tape and tree relations, text tests, aggregates, lattices and demand, <code>must, may</code>, <code>join</code>, <code>dfs_first</code>, captures over a file's source and lines, signatures and subtree digests, <code>.on syntax_error</code> and <code>.on input_error</code>, and the typing programs. Python's <code>syntax_error</code> is the verdict of a parser that follows CPython's grammar.</p>
    </div>
    <div class="tablewrap"><table class="map">
      <tr><th>Not run yet</th><th>What happens instead</th></tr>
      <tr><td>Patterns <code>n ~ `…`</code></td><td>Refused. Rules written with relations run.</td></tr>
      <tr><td>Fixes: <code>fix</code>, <code>=&gt; edits</code>, <code>.edit</code></td><td>Read and dropped: findings are reported, no edits are made.</td></tr>
      <tr><td><code>ordered by earlier</code></td><td>Refused. <code>earlier</code> and <code>visit_index</code> follow source order, not CPython's visit order.</td></tr>
      <tr><td><code>match_all</code> over a node's own text</td><td>Refused; over a file's <code>source</code> and <code>src_line</code> it runs.</td></tr>
      <tr><td>Recursion that builds a value with no bound the checker knows (a dotted name built by <code>@cat</code>)</td><td>Refused. Moves along the tree, counted steps and shrinking strings are accepted bounds.</td></tr>
      <tr><td>A <code>@place</code> anywhere but a finding's place</td><td>Refused.</td></tr>
      <tr><td>The <code>possible</code> tag and <code>.possible</code> mode</td><td>The must side is computed; possible findings are not marked yet.</td></tr>
      <tr><td>Policy file readers (<code>IO=json</code>, <code>yaml</code>, <code>lines</code>, artifacts), tsconfig, package exports and Git facts</td><td>A missing file gives an <code>input_error</code> row; a file that exists stops the run.</td></tr>
      <tr><td><code>parsed</code>, string annotations, <code>static_value</code> through constants</td><td>Empty, or the plain value.</td></tr>
      <tr><td>The text of a node over 256 bytes that is not a token, comment or string</td><td>No row: a test on it holds neither way.</td></tr>
      <tr><td><code>.mode</code>, <code>.scope</code>, <code>.budget</code>, <code>.gate</code></td><td>Parsed and kept as settings; the engine does not apply them to findings yet.</td></tr>
    </table></div>
  </section>
''')

w('''  <section class="sect">
    <div class="txt">
      <h2>Performance</h2>
      <p>This is the cost model the engine is designed to, with the measured numbers marked. It explains why rules should be written as sets. The engine runs most of the language already; the section before this one says what it does not run yet.</p>
      <p><b>How a run works.</b> Each document is parsed once into a tape, from which <code>kind</code>, <code>field</code>, <code>child</code>, <code>text</code> and the tree axes are read. The distinct strings of the run are collected, and every text test of every rule runs over them once. Rules are evaluated a stratum at a time, set at a time, in waves: a wave joins whole relations, and recursion repeats waves on the new rows only, until nothing changes. Library relations are shared by every policy in the run. Findings come last, and fixes are asked for findings only.</p>
      <p><b>What follows from it.</b></p>
      <ul class="tight">
        <li>Cost is in rows, not in lines of rule. A body that starts with <code>kind(n, "call")</code> and a name test touches a few rows; a body that starts with <code>within(n, a)</code> touches every ancestor pair.</li>
        <li>Everything is per distinct value. A text test, a regex capture or a printed node is made once per distinct string or node, however many rules ask.</li>
        <li>Nothing scans text per node. Text tests are DFA, glob or literal-set passes over strings; only <code>.scope text</code> checks read a file's lines.</li>
        <li>A relation is complete before anything negates it, counts it or passes it as an argument. Deep chains of such strata add waves, not passes over the code.</li>
        <li>Asking for one answer is cheaper than asking for all: <code>nearest</code> rather than <code>within</code>, <code>dfs_first</code> rather than a closure, the magic-set pragma rather than a whole relation read for a few rows.</li>
        <li>The working set is bounded: rows live in SQLite on disk, and the engine keeps to 1 GB of memory.</li>
      </ul>
    </div>
    <div class="tablewrap"><table class="map">
      <tr><th>Operation</th><th>Cost</th><th>Use it well</th></tr>
      <tr><td><code>kind</code>, <code>field</code>, <code>child</code>, <code>text</code>, <code>value</code></td><td>Tape lookups, indexed per document</td><td>Start bodies with a kind: it is the cheapest filter there is</td></tr>
      <tr><td><code>n ~ `pattern`</code></td><td>Matched at candidates the anchor index picks by kind and literal text</td><td>Keep a literal in the pattern (a callee name), or put a cheap atom first</td></tr>
      <tr><td><code>glob</code>, <code>prefix</code>, <code>suffix</code>, <code>contains</code>, <code>word</code>, <code>i</code> forms</td><td>Once per distinct string, all rules together: exact sets are hash sets, affix sets Aho-Corasick automata, the rest small DFAs</td><td>Test typed text (<code>name</code>, <code>str</code>) rather than the <code>text</code> of a big node; a set of patterns is a relation, and costs one automaton</td></tr>
      <tr><td><code>match</code>, captures, <code>match_all</code></td><td>A glob's DFA cost; captures after a match, once per distinct string; <code>match_all</code> linear in the text</td><td>Use a glob when one says it; <code>match</code> for captures and real grammars</td></tr>
      <tr><td><code>inside</code>, <code>has</code>, <code>nearest</code>, <code>within_stop</code>, <code>precedes_stop</code></td><td>One tape pass per axis and set</td><td>Prefer them to recursive climbs; keep the set a small relation of kinds</td></tr>
      <tr><td><code>within</code>, <code>precedes</code></td><td>Rows grow with nesting depth, or with the number of siblings</td><td>Narrow both ends first, or use the set axes</td></tr>
      <tr><td>Joins</td><td>Hash or index joins on bound columns</td><td>Bind the selective side first; a variable shared only by two large relations is a large join</td></tr>
      <tr><td><code>!r(…)</code>, <code>count : { … } = 0</code></td><td>r complete first, then an anti-join</td><td>Negate a narrow relation, not a broad one</td></tr>
      <tr><td><code>count</code>, <code>min</code>, <code>max</code>, <code>sum</code></td><td>A group-by over the body's rows</td><td>"First" and "latest" are <code>min</code>/<code>max</code> over <code>visit_index</code>, which is set-based</td></tr>
      <tr><td><code>join</code></td><td>Sorts each group</td><td>Short lists: keys, allowlists, signed messages</td></tr>
      <tr><td>Recursion</td><td>Waves over new rows only; a transitive closure can hold a row per pair of nodes</td><td>Restrict closures to the nodes rules ask about (<code>.pragma "magic-transform"</code>), or ask for one answer (<code>nearest</code>, <code>dfs_first</code>)</td></tr>
      <tr><td><code>ordered by earlier</code></td><td>One document at a time, in visit order: sequential</td><td>Only when which write counts depends on the relation itself</td></tr>
      <tr><td><code>dfs_first</code></td><td>Set passes, then a search only where chains enter cycles. <b>Measured</b> (native spike, 1,000,000 functions, 5,000,000 calls): 0.2 s of set passes, 0.06 s for every root; steps 1-3 in 0.004 s on a GPU. On LEG-083's real call graph, 1 ms</td><td>Order edges by <code>visit_index</code>; leave out edges a check does not follow, so cycles stay small</td></tr>
      <tr><td><code>dfs_first_memo</code></td><td>As <code>dfs_first</code>, plus a search in root order for roots that enter cycles. <b>Measured</b>: 0.03 s for every root, against 0.32 s for the scripts' own search</td><td>For parity with a script only</td></tr>
      <tr><td><code>must, may</code></td><td>The relation, and what negates it, twice</td><td>Only where facts can be missing</td></tr>
      <tr><td><code>str_part</code>, <code>syntax_error</code></td><td>Native, once per file: a string decoder and a parser that follows CPython's grammar. No CPython runs</td><td>Nothing to do: the cost is per file, whatever the rules ask</td></tr>
      <tr><td><code>@unparse</code></td><td>A native printer, once per node asked</td><td>Use it in messages only; compare trees with <code>@tree_digest</code></td></tr>
      <tr><td><code>parsed(s, g, root)</code></td><td>Parses the string, once per distinct string</td><td>For annotations and embedded code, not every string literal</td></tr>
      <tr><td><code>.input</code> files, artifacts</td><td>Read once per run; an artifact is checked against the digest of its inputs</td><td>Keep allowlists and baselines as files, not as facts in rules</td></tr>
      <tr><td>Fixes</td><td>Asked per finding only</td><td>Put a costly safety argument in the fix, not in the finding rule</td></tr>
    </table></div>
  </section>
''')

w('''  <section class="sect">
    <div class="txt">
      <h2>Where to look next</h2>
      <ul class="tight">
        <li><a href="#packs">Pack rules</a>: every shipped rule, its hand-written .rr beside today's JSON.</li>
        <li><a href="#data-layer">Data-layer rules</a>: hand-written final forms of complex data-layer checks, and the router example in four forms. function-level-imports, missing-async-pairs and the router carry fixes.</li>
        <li><a href="#library">Library</a>: the Datalog libraries, the native core they stand on, and the Python typing program.</li>
      </ul>
      <p class="note" style="padding:0">The pack rules run in the rrlab harness (Soufflé), scored against each tool's own findings; the engine reads them, and its replay runs the data-layer checks. The generated tab is a display translation of today's JSON.</p>
    </div>
  </section>
</div>
''')
open(os.path.join(HERE, "syntax.html"), "w").write("\n".join(out))
out = []
exec(open(os.path.join(HERE, "pages_intro.py")).read())
open(os.path.join(HERE, "intro.html"), "w").write("\n".join(out))
