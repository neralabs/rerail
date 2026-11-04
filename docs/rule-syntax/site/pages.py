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
      <tr><td><code>match</code>, <code>cat</code>, <code>strlen</code>, <code>substr</code>, <code>to_number</code>; <code>@functor(…)</code> with <code>.functor</code></td>
          <td><code>.violation</code>, <code>fix</code>, <code>=&gt; edits</code> and <code>.edit</code>: findings and fixes</td></tr>
      <tr><td><code>.decl</code>, <code>.type</code>, <code>.input</code>, <code>.output</code>, <code>.pragma</code>, <code>#include</code></td>
          <td>A relation name as the stop-set argument of <code>within_stop</code>, <code>nearest</code> and <code>precedes_stop</code>: <code>nearest(f, s, scope)</code></td></tr>
      <tr><td>Stratified negation: a relation is complete before anything negates it</td>
          <td>Policy directives: <code>.policy</code>, <code>.description</code>, <code>.language</code>, <code>.mode</code>, <code>.scope</code>, <code>.budget</code>, …; <code>#include</code> loads each file once</td></tr>
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
      <tr><td><code>.edit e(…)</code></td><td>An edit defined in Datalog, for fixes (see "Fixes").</td></tr>
    </table></div>
  </section>
''')

sect("Bodies", [
    "Each rule below is one construct, with what it matches. All of them are Soufflé. A variable inside <code>{ … }</code> stays there unless it is also used outside.",
], [
 ("rr", '''
// Regex: match tests the whole text, as in Soufflé, so `.*` stands for "anything after".
// On a node it reads the node's text. Matches <div>'s tag `div`, not <Card>'s `Card`.
host_tag(t) :- kind(t, "identifier"), match("[a-z].*", t).

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
    </div>
    <div class="tablewrap"><table class="map">
      <tr><th>JSON</th><th>.rr</th></tr>
      <tr><td><code>"kind": "call"</code></td><td><code>kind(n, "call")</code></td></tr>
      <tr><td><code>"regex": "^exec$"</code></td><td><code>match("exec", n)</code>: whole-text, so <code>^</code> and <code>$</code> go and <code>.*</code> marks open ends</td></tr>
      <tr><td><code>"regex": "(?&lt;D&gt;todo)"</code></td><td><code>match(".*(?&lt;D&gt;todo).*", n)</code>, which binds the span <code>D</code></td></tr>
      <tr><td><code>"pattern": "$F($$$)"</code></td><td><code>n ~ `$F($$$)`</code>, which binds <code>F</code></td></tr>
      <tr><td><code>"has": {"field": "f", …}</code></td><td><code>field(n, "f", c), …</code></td></tr>
      <tr><td><code>"has": {…}</code></td><td><code>child(n, c, _), …</code></td></tr>
      <tr><td><code>"has": {"stopBy": "end", …}</code></td><td><code>within(c, n), …</code></td></tr>
      <tr><td><code>"inside": {"stopBy": "end", …}</code></td><td><code>within(n, p), …</code></td></tr>
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
    "The binary provides the parse tape, tree arithmetic, value builtins, file readers and the edit applier. Name resolution, imports, call targets, JSX structure, typing and most edits are Datalog libraries over those. A library update ships without a new binary.",
    '''<ul class="tight">
        <li><code>core</code>: native. Tape relations, <code>within</code>, <code>nearest</code>, functors, config readers, the four edits.</li>
        <li><code>python/names</code>, <code>python/imports</code>, <code>python/calls</code>: adapted from an earlier Datalog prototype of Python call resolution.</li>
        <li><code>tsx/names</code>, <code>tsx/jsx</code>: drafted for this page.</li>
      </ul>''',
], [
 ("rr", '''
#include "python/names.rr"     // scope_node, binding, ref_site, sym_of, annotation, catches, …
#include "python/imports.rr"   // import_alias, loads, reaches, exports, hoist, …
#include "python/calls.rr"     // call_target, call_open, reach, call_arg, awaited, …

// core (native): kind, field, child, text, value, within, nearest, line, doc, file, tsconfig_path,
//                replace, delete, insert_before, insert_after
''', "What a Python rule can use"),
])

sect("Unknown is not false", [
    "Some facts cannot be known. Below, <code>Shell</code> comes from a package Rerail has no source for, so whether it renders its children is unknown. A two-valued rule must pick: treat unknown as false and miss real bugs, or as true and report things that may be fine.",
    "A relation declared <code>must, may</code> is computed twice. <code>must</code> holds what is proven; <code>may</code> holds what is proven or unknown. A rule prefixed <code>must</code> or <code>may</code> adds rows to that side only. A rule with no prefix adds to both.",
    "Negation swaps sides. <code>must stuck(e) :- …, !passes(e)</code> reads the may side: the element is surely stuck only if it surely does not pass. So a definite finding never rests on an unknown.",
    "A finding that holds only on the may side keeps its code, is tagged <code>possible</code>, and uses the <code>.possible</code> mode, here <code>log</code> instead of <code>block</code>.",
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

w('''  <section class="sect">
    <div class="txt">
      <h2>Where to look next</h2>
      <ul class="tight">
        <li><a href="#packs">Pack rules</a>: every shipped rule, today's JSON beside the generated .rr.</li>
        <li><a href="#data-layer">Data-layer rules</a>: hand-written final forms of complex data-layer checks, and the router example in four forms. function-level-imports, missing-async-pairs and the router carry fixes.</li>
        <li><a href="#library">Library</a>: the Datalog libraries and the native core they stand on.</li>
      </ul>
      <p class="note" style="padding:0">Generated .rr is a display translation from today's JSON. It shows the shape; nothing here has been compiled or run against the engine.</p>
    </div>
  </section>
</div>
''')
open(os.path.join(HERE, "syntax.html"), "w").write("\n".join(out))
out = []
exec(open(os.path.join(HERE, "pages_intro.py")).read())
open(os.path.join(HERE, "intro.html"), "w").write("\n".join(out))
